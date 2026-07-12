use std::path::{Path, PathBuf};
use std::time::Duration;

use clawdometer_core::state::State;
use tauri::{AppHandle, Emitter};

pub const STATE_EVENT: &str = "state-updated";

/// Merge the statusline snapshot (state.json) with the live-poll snapshot
/// (live.json). rate_limits + captured_at come from whichever snapshot is
/// newer AND actually has rate_limits; model/context only ever exist in the
/// statusline snapshot. captured_at strings are RFC3339 UTC from the same
/// formatter, so lexicographic comparison is chronological.
fn merge(state: Option<State>, live: Option<State>) -> Option<State> {
    match (state, live) {
        (Some(s), Some(l)) => {
            let use_live = l.rate_limits.is_some()
                && (s.rate_limits.is_none() || l.captured_at > s.captured_at);
            if use_live {
                Some(State { rate_limits: l.rate_limits, captured_at: l.captured_at, ..s })
            } else {
                Some(s)
            }
        }
        (s, l) => s.or(l),
    }
}

pub fn build_payload(state_path: &Path, live_path: &Path) -> serde_json::Value {
    let merged = merge(
        clawdometer_core::state::read_state(state_path),
        clawdometer_core::state::read_state(live_path),
    );
    let state = merged
        .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null))
        .unwrap_or(serde_json::Value::Null);
    let received_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    serde_json::json!({ "state": state, "received_at_ms": received_at_ms })
}

/// notify watcher on ~/.clawdometer + 2s fallback poll. Emits only when the
/// serialized payload's state differs from the last emission (poll path) or
/// the FS reports a change (watch path). Never panics the app: watch setup
/// failure degrades to poll-only.
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
        let payload = build_payload(&state_path, &live_path);
        let mut last_state = payload["state"].clone();
        let _ = app.emit(STATE_EVENT, &payload);
        update_tooltip(&app, &payload);

        // The webview's event listener attaches asynchronously, so early
        // emissions can be lost. Re-emit unconditionally for the first few
        // ticks so a restarted HUD renders pre-existing state.
        let mut startup_grace: u32 = 5;

        loop {
            // wake on FS event or every 2s (debounce fallback poll)
            let _ = rx.recv_timeout(Duration::from_secs(2));
            let payload = build_payload(&state_path, &live_path);
            if startup_grace > 0 || payload["state"] != last_state {
                startup_grace = startup_grace.saturating_sub(1);
                last_state = payload["state"].clone();
                let _ = app.emit(STATE_EVENT, &payload);
                update_tooltip(&app, &payload);
            }
        }
    });
}

fn update_tooltip(app: &AppHandle, payload: &serde_json::Value) {
    let text = match (
        payload.pointer("/state/rate_limits/five_hour/used_percentage").and_then(|v| v.as_i64()),
        payload.pointer("/state/rate_limits/seven_day/used_percentage").and_then(|v| v.as_i64()),
    ) {
        (Some(fh), Some(sd)) => format!("5h {fh}% · 7d {sd}%"),
        (Some(fh), None) => format!("5h {fh}%"),
        (None, Some(sd)) => format!("7d {sd}%"),
        (None, None) => String::from("Clawdometer — waiting for data"),
    };
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawdometer_core::schema::{LimitWindow, RateLimits};
    use clawdometer_core::state::SCHEMA_VERSION;

    fn snapshot(captured_at: &str, pct: Option<i64>) -> State {
        State {
            schema_version: SCHEMA_VERSION,
            captured_at: captured_at.into(),
            rate_limits: pct.map(|p| RateLimits {
                five_hour: Some(LimitWindow { used_percentage: p, resets_at: 0 }),
                seven_day: None,
            }),
            model: None,
            context_window: None,
            session_id: None,
            transcript_path: None,
            cli_version: None,
        }
    }

    fn five_hour_pct(s: &State) -> Option<i64> {
        s.rate_limits.as_ref()?.five_hour.as_ref().map(|w| w.used_percentage)
    }

    #[test]
    fn payload_is_null_state_when_file_missing_or_torn() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("state.json");
        let live = dir.path().join("live.json");
        assert!(build_payload(&missing, &live)["state"].is_null());
        std::fs::write(&missing, "{ torn").unwrap();
        assert!(build_payload(&missing, &live)["state"].is_null());
    }

    #[test]
    fn payload_carries_state_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let raw = include_str!("../../../crates/core/tests/fixtures/stdin-with-limits.json");
        let input = clawdometer_core::schema::parse_statusline_input(raw).unwrap();
        let state = clawdometer_core::state::State::from_input(&input, "t".into());
        clawdometer_core::state::write_state_atomic(&path, &state).unwrap();
        let payload = build_payload(&path, &dir.path().join("live.json"));
        assert_eq!(payload["state"]["rate_limits"]["five_hour"]["used_percentage"], 1);
        assert_eq!(payload["state"]["captured_at"], "t");
    }

    #[test]
    fn newer_live_rate_limits_win() {
        let s = snapshot("2026-07-12T00:00:00Z", Some(97));
        let l = snapshot("2026-07-12T00:05:00Z", Some(99));
        let m = merge(Some(s), Some(l)).unwrap();
        assert_eq!(five_hour_pct(&m), Some(99));
        assert_eq!(m.captured_at, "2026-07-12T00:05:00Z");
    }

    #[test]
    fn newer_statusline_wins_over_older_live() {
        let s = snapshot("2026-07-12T00:10:00Z", Some(97));
        let l = snapshot("2026-07-12T00:05:00Z", Some(99));
        let m = merge(Some(s), Some(l)).unwrap();
        assert_eq!(five_hour_pct(&m), Some(97));
    }

    #[test]
    fn live_fills_in_when_statusline_has_no_limits_yet() {
        let s = snapshot("2026-07-12T00:10:00Z", None);
        let l = snapshot("2026-07-12T00:05:00Z", Some(99));
        let m = merge(Some(s), Some(l)).unwrap();
        assert_eq!(five_hour_pct(&m), Some(99));
    }

    #[test]
    fn either_side_alone_passes_through() {
        assert_eq!(
            five_hour_pct(&merge(None, Some(snapshot("t", Some(4)))).unwrap()),
            Some(4)
        );
        assert_eq!(
            five_hour_pct(&merge(Some(snapshot("t", Some(7))), None).unwrap()),
            Some(7)
        );
        assert!(merge(None, None).is_none());
    }
}
