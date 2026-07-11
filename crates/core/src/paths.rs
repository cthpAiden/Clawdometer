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

pub fn default_claude_settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("settings.json")
}
