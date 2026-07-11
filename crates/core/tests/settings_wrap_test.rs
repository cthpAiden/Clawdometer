use std::path::{Path, PathBuf};

use clawdometer_core::settings::{install, InstallOutcome, SettingsError};

const OURS: &str = r#""C:\bin\clawdometer.exe" hook"#;

struct Env {
    _tmp: tempfile::TempDir,
    settings: PathBuf,
    claw: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("clawdometer");
    Env { settings, claw, _tmp: tmp }
}

fn read_json(path: &Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(raw.trim_start_matches('\u{feff}')).unwrap()
}

#[test]
fn install_with_missing_settings_creates_file() {
    let e = env();
    let outcome = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    assert_eq!(outcome, InstallOutcome::Installed);
    let json = read_json(&e.settings);
    assert_eq!(json["statusLine"]["command"], OURS);
    assert!(!e.claw.join("wrapped.json").exists());
    // no backup for a file that didn't exist
    assert!(!e.claw.join("backups").exists());
}

#[test]
fn install_with_empty_object() {
    let e = env();
    std::fs::write(&e.settings, "{}").unwrap();
    let outcome = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    assert_eq!(outcome, InstallOutcome::Installed);
    assert_eq!(read_json(&e.settings)["statusLine"]["command"], OURS);
}

#[test]
fn install_wraps_existing_statusline_preserving_extra_fields() {
    let e = env();
    std::fs::write(
        &e.settings,
        r#"{"statusLine":{"command":"my-old-line.cmd","padding":0,"custom":true},"model":"opus"}"#,
    ).unwrap();
    let outcome = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    assert_eq!(outcome, InstallOutcome::Wrapped);
    // full original object persisted, extra fields intact
    let wrapped = read_json(&e.claw.join("wrapped.json"));
    assert_eq!(wrapped["command"], "my-old-line.cmd");
    assert_eq!(wrapped["padding"], 0);
    assert_eq!(wrapped["custom"], true);
    // statusLine replaced, other keys survive
    let json = read_json(&e.settings);
    assert_eq!(json["statusLine"]["command"], OURS);
    assert_eq!(json["model"], "opus");
}

#[test]
fn install_preserves_all_other_keys_deep_equal() {
    let e = env();
    let original = r#"{
        "model": "opus",
        "permissions": {"allow": ["Bash(ls:*)"], "deny": []},
        "env": {"FOO": "bar"},
        "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "x"}]}]}
    }"#;
    std::fs::write(&e.settings, original).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let mut before: serde_json::Value = serde_json::from_str(original).unwrap();
    let mut after = read_json(&e.settings);
    before.as_object_mut().unwrap().remove("statusLine");
    after.as_object_mut().unwrap().remove("statusLine");
    assert_eq!(before, after, "non-statusLine keys must survive semantically intact");
}

#[test]
fn install_backs_up_existing_file_and_never_overwrites_backups() {
    let e = env();
    std::fs::write(&e.settings, r#"{"a":1}"#).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let b1 = e.claw.join("backups").join("settings-20260712-000000.json");
    assert_eq!(std::fs::read_to_string(&b1).unwrap(), r#"{"a":1}"#, "backup is raw original bytes");

    // force a second mutating install with the SAME timestamp
    std::fs::write(&e.settings, r#"{"a":2}"#).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let backups: Vec<_> = std::fs::read_dir(e.claw.join("backups")).unwrap().collect();
    assert_eq!(backups.len(), 2, "second backup must get a distinct name, never overwrite");
    assert_eq!(std::fs::read_to_string(&b1).unwrap(), r#"{"a":1}"#, "first backup untouched");
}

#[test]
fn install_twice_is_idempotent() {
    let e = env();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let after_first = std::fs::read_to_string(&e.settings).unwrap();
    let outcome = install(&e.settings, &e.claw, OURS, "20260712-000001").unwrap();
    assert_eq!(outcome, InstallOutcome::AlreadyInstalled);
    assert_eq!(std::fs::read_to_string(&e.settings).unwrap(), after_first, "no-op leaves file untouched");
    assert!(!e.claw.join("wrapped.json").exists(), "must not wrap our own command");
}

#[test]
fn install_aborts_on_malformed_json_touching_nothing() {
    let e = env();
    std::fs::write(&e.settings, "{ this is not json").unwrap();
    let err = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap_err();
    assert!(matches!(err, SettingsError::MalformedSettings(_)));
    assert_eq!(std::fs::read_to_string(&e.settings).unwrap(), "{ this is not json");
    assert!(!e.claw.exists(), "abort must touch nothing, not even backups");
}

#[test]
fn install_aborts_on_invalid_utf8_touching_nothing() {
    let e = env();
    // 0xFF is never valid UTF-8; embed it inside otherwise-plausible JSON bytes
    let mut bytes = br#"{"note":""#.to_vec();
    bytes.push(0xFF);
    bytes.extend_from_slice(br#""}"#);
    std::fs::write(&e.settings, &bytes).unwrap();
    let err = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap_err();
    assert!(matches!(err, SettingsError::MalformedSettings(_)));
    assert_eq!(std::fs::read(&e.settings).unwrap(), bytes, "file untouched");
    assert!(!e.claw.exists(), "abort must touch nothing");
}

#[test]
fn install_handles_bom_and_crlf() {
    let e = env();
    let content = "\u{feff}{\r\n  \"model\": \"opus\",\r\n  \"statusLine\": {\"command\": \"old.cmd\"}\r\n}\r\n";
    std::fs::write(&e.settings, content).unwrap();
    let outcome = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    assert_eq!(outcome, InstallOutcome::Wrapped);
    let json = read_json(&e.settings);
    assert_eq!(json["model"], "opus");
    assert_eq!(json["statusLine"]["command"], OURS);
    // backup preserved the original bytes exactly, BOM and CRLF included
    let backup = std::fs::read(e.claw.join("backups").join("settings-20260712-000000.json")).unwrap();
    assert_eq!(backup, content.as_bytes());
}

#[test]
fn install_preserves_unicode_values() {
    let e = env();
    std::fs::write(
        &e.settings,
        r#"{"userName":"Phúc Châu 🦀","note":"日本語テスト","statusLine":{"command":"echo héllo"}}"#,
    ).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let json = read_json(&e.settings);
    assert_eq!(json["userName"], "Phúc Châu 🦀");
    assert_eq!(json["note"], "日本語テスト");
    let wrapped = read_json(&e.claw.join("wrapped.json"));
    assert_eq!(wrapped["command"], "echo héllo");
}

#[test]
fn install_replaces_stale_clawdometer_hook_command_without_wrapping() {
    let e = env();
    std::fs::write(
        &e.settings,
        r#"{"statusLine":{"command":"\"C:\\old path\\clawdometer.exe\" hook"},"model":"opus"}"#,
    ).unwrap();
    let outcome = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    assert_eq!(outcome, InstallOutcome::Installed, "stale self-command must be Installed, not Wrapped");
    assert!(!e.claw.join("wrapped.json").exists(), "must not wrap a stale clawdometer hook command");
    let json = read_json(&e.settings);
    assert_eq!(json["statusLine"]["command"], OURS);
    assert_eq!(json["model"], "opus");
    // still backed up, since the file existed
    assert!(e.claw.join("backups").join("settings-20260712-000000.json").exists());
}

#[test]
fn extra_fields_survive_full_wrap_round_trip() {
    // install (wrap) -> uninstall (restore) must return the EXACT original object.
    // The uninstall half of this assertion lives in settings_uninstall_test.rs;
    // here we prove wrapped.json captures the full object.
    let e = env();
    let original_status_line = serde_json::json!({
        "command": "old.cmd", "padding": 2, "type": "command", "nested": {"deep": [1, 2, 3]}
    });
    let root = serde_json::json!({ "statusLine": original_status_line });
    std::fs::write(&e.settings, serde_json::to_string(&root).unwrap()).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    assert_eq!(read_json(&e.claw.join("wrapped.json")), original_status_line);
}
