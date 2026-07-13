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

/// Refresh snapshot written by the HUD after a headless `claude -p /usage`
/// run (same State shape as state.json, rate_limits only). The official CLI
/// fetches the numbers with its own credentials; Clawdometer only parses the
/// text it prints.
pub fn live_path() -> PathBuf {
    clawdometer_dir().join("live.json")
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

pub fn default_claude_settings_path() -> PathBuf {
    claude_config_dir().join("settings.json")
}

/// Claude Code's transcript root: <config>/projects, holding one
/// <encoded-cwd>/<session>.jsonl per session. Claude Code appends to these as a
/// session generates — on every client, including GUIs that never invoke the
/// statusline hook — so their mtime is the HUD's cross-client activity signal.
pub fn projects_dir() -> PathBuf {
    claude_config_dir().join("projects")
}

/// Absolute path to a System32 executable (cmd.exe, taskkill.exe).
/// Spawning by bare name lets Windows resolve via the application directory
/// and PATH before System32 — an absolute path removes that planting surface.
/// Falls back to the bare name (normal search) only if SystemRoot is unset,
/// which effectively never happens on Windows.
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
