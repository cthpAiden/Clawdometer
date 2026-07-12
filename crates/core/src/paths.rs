use std::path::PathBuf;

/// ~/.clawdometer, overridable via CLAWDOMETER_DIR (used by tests; never
/// point it at a real ~/.claude directory).
pub fn clawdometer_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAWDOMETER_DIR") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".clawdometer")
}

pub fn state_path() -> PathBuf {
    clawdometer_dir().join("state.json")
}

pub fn wrapped_path() -> PathBuf {
    clawdometer_dir().join("wrapped.json")
}

/// Live-poll snapshot written by the HUD's usage poller (same State shape as
/// state.json, rate_limits only).
pub fn live_path() -> PathBuf {
    clawdometer_dir().join("live.json")
}

pub fn claude_credentials_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join(".credentials.json")
}

pub fn default_claude_settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("settings.json")
}
