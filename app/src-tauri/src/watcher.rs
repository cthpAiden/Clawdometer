use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

pub const STATE_EVENT: &str = "state-updated";

pub fn build_payload(state_path: &Path) -> serde_json::Value {
    let state = clawdometer_core::state::read_state(state_path)
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
        let payload = build_payload(&state_path);
        let mut last_state = payload["state"].clone();
        let _ = app.emit(STATE_EVENT, &payload);

        loop {
            // wake on FS event or every 2s (debounce fallback poll)
            let _ = rx.recv_timeout(Duration::from_secs(2));
            let payload = build_payload(&state_path);
            if payload["state"] != last_state {
                last_state = payload["state"].clone();
                let _ = app.emit(STATE_EVENT, &payload);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_null_state_when_file_missing_or_torn() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("state.json");
        assert!(build_payload(&missing)["state"].is_null());
        std::fs::write(&missing, "{ torn").unwrap();
        assert!(build_payload(&missing)["state"].is_null());
    }

    #[test]
    fn payload_carries_state_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let raw = include_str!("../../../crates/core/tests/fixtures/stdin-with-limits.json");
        let input = clawdometer_core::schema::parse_statusline_input(raw).unwrap();
        let state = clawdometer_core::state::State::from_input(&input, "t".into());
        clawdometer_core::state::write_state_atomic(&path, &state).unwrap();
        let payload = build_payload(&path);
        assert_eq!(payload["state"]["rate_limits"]["five_hour"]["used_percentage"], 1);
        assert_eq!(payload["state"]["captured_at"], "t");
    }
}
