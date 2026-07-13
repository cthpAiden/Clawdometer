use std::path::{Path, PathBuf};
use std::time::Duration;

use clawdometer_core::state::{merge, zero_expired_windows};
use tauri::{AppHandle, Emitter};

pub const STATE_EVENT: &str = "state-updated";

pub fn build_payload(
    state_path: &Path,
    live_path: &Path,
    now_epoch_secs: i64,
) -> serde_json::Value {
    // state.json (statusline hook) merged with live.json (headless /usage
    // refresh) — whichever snapshot is newer wins.
    let state = merge(
        clawdometer_core::state::read_state(state_path),
        clawdometer_core::state::read_state(live_path),
    )
    .map(|mut s| {
        // A window whose reset time has passed is 0% until the next request
        // opens a new one — without this, an idle machine would show the last
        // snapshot's percentage forever.
        if let Some(rl) = s.rate_limits.as_mut() {
            zero_expired_windows(rl, now_epoch_secs);
        }
        s
    });
    // The webview gets only what it renders: rate_limits + captured_at.
    // session_id / transcript_path / model / cli_version stay on this side
    // of the IPC boundary.
    let state = state
        .map(|s| serde_json::json!({ "captured_at": s.captured_at, "rate_limits": s.rate_limits }))
        .unwrap_or(serde_json::Value::Null);
    serde_json::json!({ "state": state })
}

fn now_epoch_secs() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// notify watcher on ~/.clawdometer + 2s fallback poll. Emits only when the
/// serialized payload's state differs from the last emission (poll path) or
/// the FS reports a change (watch path) — zeroing makes the payload cross a
/// window's reset time exactly once, so that transition re-emits too. Never
/// panics the app: watch setup failure degrades to poll-only.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let state_path: PathBuf = clawdometer_core::paths::state_path();
        let live_path: PathBuf = clawdometer_core::paths::live_path();
        let dir = state_path.parent().map(Path::to_path_buf);

        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let _watcher = dir.and_then(|dir| {
            use notify::Watcher;
            let tx = tx.clone();
            let mut w = notify::recommended_watcher(move |_res| {
                let _ = tx.send(());
            })
            .ok()?;
            std::fs::create_dir_all(&dir).ok();
            w.watch(&dir, notify::RecursiveMode::NonRecursive).ok()?;
            Some(w)
        });

        // initial emission so the UI renders immediately
        let payload = build_payload(&state_path, &live_path, now_epoch_secs());
        let mut last_payload = payload.clone();
        let _ = app.emit(STATE_EVENT, &payload);
        update_tooltip(&app, &payload);

        // The webview's event listener attaches asynchronously, so early
        // emissions can be lost. Re-emit unconditionally for the first few
        // ticks so a restarted HUD renders pre-existing state.
        let mut startup_grace: u32 = 5;

        loop {
            // wake on FS event or every 2s (debounce fallback poll)
            let _ = rx.recv_timeout(Duration::from_secs(2));
            let payload = build_payload(&state_path, &live_path, now_epoch_secs());
            // The payload is timestamp-free, so identical data never re-emits.
            if startup_grace > 0 || payload != last_payload {
                startup_grace = startup_grace.saturating_sub(1);
                last_payload = payload.clone();
                let _ = app.emit(STATE_EVENT, &payload);
                update_tooltip(&app, &payload);
            }
        }
    });
}

/// Tray tooltip: percentages when there's data; otherwise how to get some —
/// the statusline hook only delivers data while Claude Code runs, so a hidden
/// HUD isn't the only place a first run is explained.
fn tooltip_text(payload: &serde_json::Value) -> String {
    match (
        payload.pointer("/state/rate_limits/five_hour/used_percentage").and_then(|v| v.as_i64()),
        payload.pointer("/state/rate_limits/seven_day/used_percentage").and_then(|v| v.as_i64()),
    ) {
        (Some(fh), Some(sd)) => format!("5h {fh}% · 7d {sd}%"),
        (Some(fh), None) => format!("5h {fh}%"),
        (None, Some(sd)) => format!("7d {sd}%"),
        (None, None) => "Clawdometer — waiting for data, open Claude Code".into(),
    }
}

fn update_tooltip(app: &AppHandle, payload: &serde_json::Value) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(tooltip_text(payload)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawdometer_core::schema::{LimitWindow, RateLimits};
    use clawdometer_core::state::{State, SCHEMA_VERSION};

    fn snapshot(captured_at: &str, pct: Option<i64>, resets_at: i64) -> State {
        State {
            schema_version: SCHEMA_VERSION,
            captured_at: captured_at.into(),
            rate_limits: pct.map(|p| RateLimits {
                five_hour: Some(LimitWindow { used_percentage: p, resets_at }),
                seven_day: None,
            }),
            model: None,
            context_window: None,
            session_id: None,
            transcript_path: None,
            cli_version: None,
        }
    }

    #[test]
    fn payload_is_null_state_when_file_missing_or_torn() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("state.json");
        let live = dir.path().join("live.json");
        assert!(build_payload(&missing, &live, 0)["state"].is_null());
        std::fs::write(&missing, "{ torn").unwrap();
        assert!(build_payload(&missing, &live, 0)["state"].is_null());
    }

    #[test]
    fn payload_carries_state_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let raw = include_str!("../../../crates/core/tests/fixtures/stdin-with-limits.json");
        let input = clawdometer_core::schema::parse_statusline_input(raw).unwrap();
        let state = clawdometer_core::state::State::from_input(&input, "t".into());
        clawdometer_core::state::write_state_atomic(&path, &state).unwrap();
        let payload = build_payload(&path, &dir.path().join("live.json"), 0);
        assert_eq!(payload["state"]["rate_limits"]["five_hour"]["used_percentage"], 1);
        assert_eq!(payload["state"]["captured_at"], "t");
    }

    #[test]
    fn newer_live_refresh_snapshot_wins() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let live_path = dir.path().join("live.json");
        let s = snapshot("2026-07-12T00:00:00Z", Some(97), 9_999_999_999);
        let l = snapshot("2026-07-12T00:05:00Z", Some(99), 9_999_999_999);
        clawdometer_core::state::write_state_atomic(&state_path, &s).unwrap();
        clawdometer_core::state::write_state_atomic(&live_path, &l).unwrap();
        let payload = build_payload(&state_path, &live_path, 0);
        assert_eq!(payload["state"]["rate_limits"]["five_hour"]["used_percentage"], 99);
        assert_eq!(payload["state"]["captured_at"], "2026-07-12T00:05:00Z");
    }

    #[test]
    fn payload_exposes_only_what_the_ui_renders() {
        // The webview should never receive session_id / transcript_path /
        // model / cli_version — it renders rate_limits + captured_at only.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = snapshot("t", Some(42), 9_999_999_999);
        state.session_id = Some("sess-secret".into());
        state.transcript_path = Some("C:\\transcripts\\x.jsonl".into());
        state.cli_version = Some("2.1.205".into());
        clawdometer_core::state::write_state_atomic(&path, &state).unwrap();
        let payload = build_payload(&path, &dir.path().join("live.json"), 0);
        let obj = payload["state"].as_object().unwrap();
        assert_eq!(obj.keys().collect::<Vec<_>>(), ["captured_at", "rate_limits"]);
        assert!(payload.get("poll_error").is_none(), "dead field must be gone");
    }

    #[test]
    fn payload_zeroes_window_once_its_reset_time_passes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let live = dir.path().join("live.json");
        let state = snapshot("t", Some(42), 1_000);
        clawdometer_core::state::write_state_atomic(&path, &state).unwrap();
        // before the reset: the snapshot's percentage
        let before = build_payload(&path, &live, 999);
        assert_eq!(before["state"]["rate_limits"]["five_hour"]["used_percentage"], 42);
        // at/after the reset: derived 0%, resets_at kept for the UI label
        let after = build_payload(&path, &live, 1_000);
        assert_eq!(after["state"]["rate_limits"]["five_hour"]["used_percentage"], 0);
        assert_eq!(after["state"]["rate_limits"]["five_hour"]["resets_at"], 1_000);
        // the transition changes the payload exactly once, so the watcher
        // loop's compare re-emits at the boundary
        assert_ne!(before, after);
        assert_eq!(after, build_payload(&path, &live, 2_000));
    }

    #[test]
    fn tooltip_explains_how_to_get_data() {
        let t = tooltip_text(&serde_json::json!({ "state": null }));
        assert_eq!(t, "Clawdometer — waiting for data, open Claude Code");
    }

    #[test]
    fn tooltip_shows_percentages_when_present() {
        let payload = serde_json::json!({
            "state": { "rate_limits": {
                "five_hour": { "used_percentage": 4 },
                "seven_day": { "used_percentage": 16 },
            }},
        });
        assert_eq!(tooltip_text(&payload), "5h 4% · 7d 16%");
    }
}
