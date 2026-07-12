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

/// Poller failure kind (`{"kind": "auth" | "network" | "no-credentials" |
/// "no-curl"}`), written by the HUD's usage poller so the UI can hint at the
/// right recovery. Deleted on success.
pub fn poll_error_path() -> PathBuf {
    clawdometer_dir().join("poll_error.json")
}

/// Claude Code's config dir: CLAUDE_CONFIG_DIR if set (Claude Code honors it
/// to relocate ~/.claude), else ~/.claude.
fn claude_config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
}

pub fn claude_credentials_path() -> PathBuf {
    claude_config_dir().join(".credentials.json")
}

pub fn default_claude_settings_path() -> PathBuf {
    claude_config_dir().join("settings.json")
}

/// Absolute path to a System32 executable (cmd.exe, taskkill.exe, curl.exe).
/// Spawning by bare name lets Windows resolve via the application directory
/// and PATH before System32 — an absolute path removes that planting surface
/// (the curl call carries the OAuth token). Falls back to the bare name
/// (normal search) only if SystemRoot is unset, which effectively never
/// happens on Windows.
pub fn system32_exe(name: &str) -> PathBuf {
    match std::env::var_os("SystemRoot") {
        Some(root) => PathBuf::from(root).join("System32").join(name),
        None => PathBuf::from(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn system32_exe_is_absolute_under_system_root() {
        let p = system32_exe("cmd.exe");
        assert!(p.is_absolute(), "must not rely on PATH/app-dir search: {p:?}");
        assert!(p.ends_with("System32\\cmd.exe"), "{p:?}");
        assert!(p.exists(), "resolved path must actually exist: {p:?}");
    }
}
