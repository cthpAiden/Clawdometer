use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub const STATUSLINE_KEY: &str = "statusLine";

#[derive(Debug, PartialEq)]
pub enum InstallOutcome {
    Installed,
    Wrapped,
    AlreadyInstalled,
}

#[derive(Debug)]
pub enum SettingsError {
    MalformedSettings(String),
    Io(std::io::Error),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::MalformedSettings(msg) => {
                write!(f, "settings.json is not valid JSON — nothing was changed: {msg}")
            }
            SettingsError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl From<std::io::Error> for SettingsError {
    fn from(e: std::io::Error) -> Self {
        SettingsError::Io(e)
    }
}

/// (parsed root object, file existed, raw original bytes)
fn load_settings(path: &Path) -> Result<(Value, bool, Vec<u8>), SettingsError> {
    if !path.exists() {
        return Ok((serde_json::json!({}), false, Vec::new()));
    }
    let raw = std::fs::read(path)?;
    let text = std::str::from_utf8(&raw)
        .map_err(|_| SettingsError::MalformedSettings("settings.json is not valid UTF-8".into()))?;
    let root: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| SettingsError::MalformedSettings(e.to_string()))?;
    if !root.is_object() {
        return Err(SettingsError::MalformedSettings("root is not a JSON object".into()));
    }
    Ok((root, true, raw))
}

fn atomic_write(path: &Path, body: &[u8]) -> Result<(), SettingsError> {
    let dir = path.parent().ok_or_else(|| {
        SettingsError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "no parent dir"))
    })?;
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(body)?;
    tmp.persist(path).map_err(|e| SettingsError::Io(e.error))?;
    Ok(())
}

fn save_settings(path: &Path, root: &Value) -> Result<(), SettingsError> {
    let body = serde_json::to_string_pretty(root)
        .map_err(|e| SettingsError::MalformedSettings(e.to_string()))?;
    atomic_write(path, body.as_bytes())
}

/// Raw-bytes backup; never overwrites an existing backup. create_new makes
/// the exists-check-then-write race-free against a concurrent install.
fn backup(clawdometer_dir: &Path, timestamp: &str, raw: &[u8]) -> Result<PathBuf, SettingsError> {
    let dir = clawdometer_dir.join("backups");
    std::fs::create_dir_all(&dir)?;
    let mut n = 0;
    loop {
        let candidate = if n == 0 {
            dir.join(format!("settings-{timestamp}.json"))
        } else {
            dir.join(format!("settings-{timestamp}-{n}.json"))
        };
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(mut f) => {
                f.write_all(raw)?;
                return Ok(candidate);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e.into()),
        }
    }
}

fn is_ours(status_line: &Value, our_command: &str) -> bool {
    status_line.get("command").and_then(|c| c.as_str()) == Some(our_command)
}

/// True if `cmd` looks like a clawdometer hook invocation, i.e. `"<path>" hook`
/// (quoted, our own format) or `<path> hook` (unquoted), where the path's file
/// stem starts with "clawdometer" (case-insensitive) — covers both the CLI
/// (`clawdometer.exe`) and the HUD binary (`clawdometer-app.exe`), which can
/// each install the hook. Used to detect a stale clawdometer command left over
/// from an install at a different exe path, so we never wrap it (which would
/// make the hook chain-call itself).
pub fn is_clawdometer_hook_command(cmd: &str) -> bool {
    let cmd = cmd.trim();
    let Some(rest) = cmd.strip_suffix(" hook") else {
        return false;
    };
    let rest = rest.trim();
    let path_str = if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
        &rest[1..rest.len() - 1]
    } else {
        rest
    };
    if path_str.is_empty() {
        return false;
    }
    Path::new(path_str)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|stem| stem.to_ascii_lowercase().starts_with("clawdometer"))
        .unwrap_or(false)
}

pub fn install(
    settings_path: &Path,
    clawdometer_dir: &Path,
    our_command: &str,
    timestamp: &str,
) -> Result<InstallOutcome, SettingsError> {
    let (mut root, existed, raw) = load_settings(settings_path)?;

    if let Some(existing) = root.get(STATUSLINE_KEY) {
        if is_ours(existing, our_command) {
            return Ok(InstallOutcome::AlreadyInstalled);
        }
        // Stale clawdometer hook command (e.g. install moved to a new exe path):
        // it's ours, just outdated. Replace it directly — never wrap our own
        // hook command, or the hook would chain-call itself.
        let is_stale_self = existing
            .get("command")
            .and_then(|c| c.as_str())
            .map(is_clawdometer_hook_command)
            .unwrap_or(false);
        if is_stale_self {
            if existed {
                backup(clawdometer_dir, timestamp, &raw)?;
            }
            root[STATUSLINE_KEY] = serde_json::json!({ "type": "command", "command": our_command });
            save_settings(settings_path, &root)?;
            return Ok(InstallOutcome::Installed);
        }
        // Persist FULL original statusLine object (command + extra fields).
        let existing = existing.clone();
        if existed {
            backup(clawdometer_dir, timestamp, &raw)?;
        }
        std::fs::create_dir_all(clawdometer_dir)?;
        let wrapped_body = serde_json::to_string_pretty(&existing)
            .map_err(|e| SettingsError::MalformedSettings(e.to_string()))?;
        atomic_write(&clawdometer_dir.join("wrapped.json"), wrapped_body.as_bytes())?;
        root[STATUSLINE_KEY] = serde_json::json!({ "type": "command", "command": our_command });
        save_settings(settings_path, &root)?;
        return Ok(InstallOutcome::Wrapped);
    }

    if existed {
        backup(clawdometer_dir, timestamp, &raw)?;
    }
    // No statusLine key: nothing to wrap, so a leftover wrapped.json (from an
    // uninstall that couldn't delete it, or a user who removed statusLine by
    // hand) is stale — the hook would chain a statusline the user no longer
    // has. Best-effort removal, same tolerance as uninstall's cleanup.
    let _ = std::fs::remove_file(clawdometer_dir.join("wrapped.json"));
    root[STATUSLINE_KEY] = serde_json::json!({ "type": "command", "command": our_command });
    save_settings(settings_path, &root)?;
    Ok(InstallOutcome::Installed)
}

#[derive(Debug, PartialEq)]
pub enum UninstallOutcome {
    Restored,
    RemovedKey,
    NotInstalled,
    NotOurs,
}

pub fn uninstall(
    settings_path: &Path,
    clawdometer_dir: &Path,
    our_command: &str,
) -> Result<UninstallOutcome, SettingsError> {
    let (mut root, existed, _raw) = load_settings(settings_path)?;
    if !existed {
        return Ok(UninstallOutcome::NotInstalled);
    }
    let Some(current) = root.get(STATUSLINE_KEY) else {
        return Ok(UninstallOutcome::NotInstalled);
    };
    // A stale clawdometer hook (e.g. exe moved since install) is still ours,
    // even though its literal command string no longer matches our_command —
    // mirrors the stale-self detection in install(), so a moved binary can
    // still uninstall/restore instead of being mistaken for a user edit.
    let is_stale_self = current
        .get("command")
        .and_then(|c| c.as_str())
        .map(is_clawdometer_hook_command)
        .unwrap_or(false);
    if !is_ours(current, our_command) && !is_stale_self {
        // User edited statusLine after install: warn, touch nothing.
        return Ok(UninstallOutcome::NotOurs);
    }
    let wrapped_path = clawdometer_dir.join("wrapped.json");
    if wrapped_path.exists() {
        let raw = std::fs::read_to_string(&wrapped_path)?;
        let original: Value = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
            .map_err(|e| SettingsError::MalformedSettings(format!("wrapped.json: {e}")))?;
        root[STATUSLINE_KEY] = original;
        save_settings(settings_path, &root)?;
        // settings.json is already restored at this point; wrapped.json is only
        // leftover cleanup. If it can't be deleted (e.g. locked by AV/indexer on
        // Windows), don't report failure — a later uninstall will see NotOurs,
        // which is safe.
        let _ = std::fs::remove_file(&wrapped_path);
        Ok(UninstallOutcome::Restored)
    } else {
        root.as_object_mut()
            .expect("load_settings guarantees object")
            .remove(STATUSLINE_KEY);
        save_settings(settings_path, &root)?;
        Ok(UninstallOutcome::RemovedKey)
    }
}
