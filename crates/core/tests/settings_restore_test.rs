use std::path::{Path, PathBuf};

use clawdometer_core::settings::{install, uninstall, UninstallOutcome};

const OURS: &str = r#""C:\bin\clawdometer.exe" hook"#;

struct Env {
    _tmp: tempfile::TempDir,
    settings: PathBuf,
    claw: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    Env {
        settings: tmp.path().join("settings.json"),
        claw: tmp.path().join("clawdometer"),
        _tmp: tmp,
    }
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(
        std::fs::read_to_string(path).unwrap().trim_start_matches('\u{feff}'),
    ).unwrap()
}

#[test]
fn uninstall_restores_wrapped_original_exactly() {
    let e = env();
    let original = r#"{"model":"opus","statusLine":{"command":"old.cmd","padding":2,"nested":{"a":[1]}}}"#;
    std::fs::write(&e.settings, original).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();

    let outcome = uninstall(&e.settings, &e.claw, OURS).unwrap();
    assert_eq!(outcome, UninstallOutcome::Restored);

    let before: serde_json::Value = serde_json::from_str(original).unwrap();
    assert_eq!(read_json(&e.settings), before, "settings must round-trip deep-equal");
    assert!(!e.claw.join("wrapped.json").exists(), "wrapped.json consumed on restore");
}

#[test]
fn uninstall_removes_key_when_nothing_was_wrapped() {
    let e = env();
    std::fs::write(&e.settings, r#"{"model":"opus"}"#).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let outcome = uninstall(&e.settings, &e.claw, OURS).unwrap();
    assert_eq!(outcome, UninstallOutcome::RemovedKey);
    let json = read_json(&e.settings);
    assert!(json.get("statusLine").is_none());
    assert_eq!(json["model"], "opus");
}

#[test]
fn uninstall_without_install_is_not_installed() {
    let e = env();
    std::fs::write(&e.settings, r#"{"model":"opus"}"#).unwrap();
    let outcome = uninstall(&e.settings, &e.claw, OURS).unwrap();
    assert_eq!(outcome, UninstallOutcome::NotInstalled);
    assert_eq!(read_json(&e.settings)["model"], "opus");
}

#[test]
fn uninstall_with_missing_settings_is_not_installed_and_creates_nothing() {
    let e = env();
    let outcome = uninstall(&e.settings, &e.claw, OURS).unwrap();
    assert_eq!(outcome, UninstallOutcome::NotInstalled);
    assert!(!e.settings.exists(), "uninstall must never create settings.json");
}

#[test]
fn uninstall_after_user_edit_warns_and_touches_nothing() {
    let e = env();
    std::fs::write(&e.settings, r#"{"statusLine":{"command":"old.cmd"}}"#).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    // user manually edits statusLine after install
    std::fs::write(&e.settings, r#"{"statusLine":{"command":"user-new-thing.cmd"}}"#).unwrap();
    let outcome = uninstall(&e.settings, &e.claw, OURS).unwrap();
    assert_eq!(outcome, UninstallOutcome::NotOurs);
    assert_eq!(read_json(&e.settings)["statusLine"]["command"], "user-new-thing.cmd");
    assert!(e.claw.join("wrapped.json").exists(), "wrapped.json left for manual recovery");
}

#[test]
fn uninstall_aborts_on_malformed_settings() {
    let e = env();
    std::fs::write(&e.settings, "{ nope").unwrap();
    assert!(uninstall(&e.settings, &e.claw, OURS).is_err());
    assert_eq!(std::fs::read_to_string(&e.settings).unwrap(), "{ nope");
}

#[test]
fn uninstall_recognizes_stale_clawdometer_hook_command_after_exe_move() {
    // Installed from an old exe path, then the binary moved (e.g. reinstalled
    // elsewhere) — the running exe's `our_command()` no longer matches the
    // literal command string in settings.json, but it's still clearly a
    // clawdometer hook. uninstall() must recognize that, not treat it as a
    // user edit.
    let e = env();
    let original = r#"{"model":"opus","statusLine":{"command":"old.cmd"}}"#;
    std::fs::write(&e.settings, original).unwrap();
    let old_exe_ours = r#""C:\old path\clawdometer.exe" hook"#;
    install(&e.settings, &e.claw, old_exe_ours, "20260712-000000").unwrap();

    let new_exe_ours = r#""D:\new path\clawdometer.exe" hook"#;
    let outcome = uninstall(&e.settings, &e.claw, new_exe_ours).unwrap();
    assert_eq!(outcome, UninstallOutcome::Restored);

    let before: serde_json::Value = serde_json::from_str(original).unwrap();
    assert_eq!(read_json(&e.settings), before, "settings must round-trip deep-equal");
}

#[test]
fn uninstall_with_malformed_wrapped_json_removes_key_and_keeps_backup() {
    // The backup can't be restored, but a hard fail would leave the hook
    // installed until the user deletes wrapped.json by hand. Uninstall must
    // still uninstall: remove the key, keep the corrupt backup for recovery.
    let e = env();
    std::fs::write(&e.settings, r#"{"model":"opus","statusLine":{"command":"old.cmd"}}"#).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    // corrupt the wrapped backup
    std::fs::write(e.claw.join("wrapped.json"), "{ nope").unwrap();
    let outcome = uninstall(&e.settings, &e.claw, OURS).unwrap();
    assert_eq!(outcome, UninstallOutcome::RemovedKeyBackupUnreadable);
    let after = read_json(&e.settings);
    assert!(after.get("statusLine").is_none(), "statusLine key removed");
    assert_eq!(after["model"], "opus", "unrelated settings untouched");
    assert!(e.claw.join("wrapped.json").exists(), "corrupt wrapped.json left for inspection");
}
