use std::io::Read;

use clawdometer_core::paths;
use clawdometer_core::schema::parse_statusline_input;
use clawdometer_core::state::{now_rfc3339, render_statusline, write_state_atomic, State};

const FALLBACK_LINE: &str = "clawdometer: waiting";

/// Infallible: every failure path still returns a printable line.
pub fn run_hook() -> String {
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return FALLBACK_LINE.into();
    }
    let input = match parse_statusline_input(&raw) {
        Ok(input) => input,
        Err(_) => return FALLBACK_LINE.into(),
    };
    let state = State::from_input(&input, now_rfc3339());
    // Write failure must not break the statusline; HUD just stays stale.
    let _ = write_state_atomic(&paths::state_path(), &state);
    render_statusline(&state)
}
