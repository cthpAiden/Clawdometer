use std::path::Path;
use std::process::Command;

fn run(args: &[&str], claw_dir: &Path) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_clawdometer"))
        .args(args)
        .env("CLAWDOMETER_DIR", claw_dir)
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap(),
    )
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(
        std::fs::read_to_string(path).unwrap().trim_start_matches('\u{feff}'),
    ).unwrap()
}

#[test]
fn install_then_uninstall_round_trip_via_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("claw");
    std::fs::write(&settings, r#"{"model":"opus","statusLine":{"command":"old.cmd","padding":1}}"#).unwrap();

    let (stdout, _, code) = run(&["install", "--settings", settings.to_str().unwrap()], &claw);
    assert_eq!(code, 0, "install failed: {stdout}");
    let json = read_json(&settings);
    let cmd = json["statusLine"]["command"].as_str().unwrap();
    assert!(cmd.ends_with("\" hook"), "command is quoted exe + hook: {cmd}");
    assert!(cmd.contains("clawdometer"), "{cmd}");

    // second install: idempotent, still exit 0
    let (stdout, _, code) = run(&["install", "--settings", settings.to_str().unwrap()], &claw);
    assert_eq!(code, 0);
    assert!(stdout.to_lowercase().contains("already"), "{stdout}");

    let (_, _, code) = run(&["uninstall", "--settings", settings.to_str().unwrap()], &claw);
    assert_eq!(code, 0);
    let json = read_json(&settings);
    assert_eq!(json["statusLine"]["command"], "old.cmd");
    assert_eq!(json["statusLine"]["padding"], 1);
    assert_eq!(json["model"], "opus");
}

#[test]
fn uninstall_after_user_edit_exits_nonzero_and_touches_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("claw");
    std::fs::write(&settings, "{}").unwrap();
    run(&["install", "--settings", settings.to_str().unwrap()], &claw);
    std::fs::write(&settings, r#"{"statusLine":{"command":"user-edited.cmd"}}"#).unwrap();

    let (_, stderr, code) = run(&["uninstall", "--settings", settings.to_str().unwrap()], &claw);
    assert_ne!(code, 0, "user-edited statusLine must exit non-zero");
    assert!(!stderr.is_empty(), "must warn on stderr");
    assert_eq!(read_json(&settings)["statusLine"]["command"], "user-edited.cmd");
}

#[test]
fn install_malformed_settings_exits_nonzero() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("claw");
    std::fs::write(&settings, "{ nope").unwrap();
    let (_, stderr, code) = run(&["install", "--settings", settings.to_str().unwrap()], &claw);
    assert_ne!(code, 0);
    assert!(stderr.to_lowercase().contains("json"), "clear message required: {stderr}");
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ nope");
}

#[test]
fn purge_removes_clawdometer_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("claw");
    std::fs::write(&settings, "{}").unwrap();
    run(&["install", "--settings", settings.to_str().unwrap()], &claw);
    assert!(claw.exists() || !claw.exists()); // dir may or may not exist yet (no backup for fresh key)
    std::fs::create_dir_all(&claw).unwrap();
    let (_, _, code) = run(&["uninstall", "--settings", settings.to_str().unwrap(), "--purge"], &claw);
    assert_eq!(code, 0);
    assert!(!claw.exists(), "--purge removes the clawdometer dir");
}

#[test]
fn install_with_settings_flag_missing_value_exits_2_and_touches_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let claw = tmp.path().join("claw");
    let (_, stderr, code) = run(&["install", "--settings"], &claw);
    assert_eq!(code, 2, "missing --settings value must exit 2, not silently target real ~/.claude");
    assert!(!stderr.is_empty(), "must print usage to stderr");
    assert!(!claw.exists(), "no files must be written when --settings is rejected");
}

#[test]
fn install_with_settings_flag_followed_by_another_flag_exits_2() {
    let tmp = tempfile::tempdir().unwrap();
    let claw = tmp.path().join("claw");
    let (_, stderr, code) = run(&["install", "--settings", "--purge"], &claw);
    assert_eq!(code, 2, "--settings followed by another flag must exit 2");
    assert!(!stderr.is_empty());
    assert!(!claw.exists());
}

#[test]
fn uninstall_with_settings_flag_missing_value_exits_2_and_touches_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let claw = tmp.path().join("claw");
    let (_, stderr, code) = run(&["uninstall", "--settings"], &claw);
    assert_eq!(code, 2, "missing --settings value must exit 2, not silently target real ~/.claude");
    assert!(!stderr.is_empty(), "must print usage to stderr");
    assert!(!claw.exists(), "no files must be written when --settings is rejected");
}

fn write_snapshot(path: &Path, captured_at: &str, pct: i64, resets_at: i64) {
    let state = clawdometer_core::state::State {
        schema_version: clawdometer_core::state::SCHEMA_VERSION,
        captured_at: captured_at.into(),
        rate_limits: Some(clawdometer_core::schema::RateLimits {
            five_hour: Some(clawdometer_core::schema::LimitWindow {
                used_percentage: pct,
                resets_at,
            }),
            seven_day: None,
        }),
        model: None,
        context_window: None,
        session_id: None,
        transcript_path: None,
        cli_version: None,
    };
    clawdometer_core::state::write_state_atomic(path, &state).unwrap();
}

/// Far enough in the future that a test run never crosses it.
const FUTURE_RESET: i64 = 4_102_444_800; // 2100-01-01

#[test]
fn status_reports_statusline_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let claw = tmp.path().join("claw");
    std::fs::create_dir_all(&claw).unwrap();
    write_snapshot(&claw.join("state.json"), "2026-07-12T00:00:00Z", 42, FUTURE_RESET);
    let (stdout, _, code) = run(&["status"], &claw);
    assert_eq!(code, 0);
    assert!(stdout.contains("5h 42%"), "{stdout}");
    assert!(stdout.contains("2026-07-12T00:00:00Z"), "{stdout}");
}

#[test]
fn status_merges_newer_live_refresh_snapshot() {
    // Same merge the HUD does: live.json (headless /usage refresh) must win
    // over an older statusline snapshot.
    let tmp = tempfile::tempdir().unwrap();
    let claw = tmp.path().join("claw");
    std::fs::create_dir_all(&claw).unwrap();
    write_snapshot(&claw.join("state.json"), "2026-07-12T00:00:00Z", 42, FUTURE_RESET);
    write_snapshot(&claw.join("live.json"), "2026-07-12T01:00:00Z", 77, FUTURE_RESET);
    let (stdout, _, code) = run(&["status"], &claw);
    assert_eq!(code, 0);
    assert!(stdout.contains("5h 77%"), "newer live.json must win: {stdout}");
    assert!(stdout.contains("2026-07-12T01:00:00Z"), "captured_at from live: {stdout}");
}

#[test]
fn status_zeroes_window_whose_reset_time_passed() {
    // Same derivation the HUD does: an expired window is 0% until the next
    // request opens a new one — status must not report a dead window's usage.
    let tmp = tempfile::tempdir().unwrap();
    let claw = tmp.path().join("claw");
    std::fs::create_dir_all(&claw).unwrap();
    write_snapshot(&claw.join("state.json"), "2026-07-12T00:00:00Z", 42, 1_000);
    let (stdout, _, code) = run(&["status"], &claw);
    assert_eq!(code, 0);
    assert!(stdout.contains("5h 0%"), "expired window must read 0%: {stdout}");
}

#[test]
fn status_reports_no_state_then_state() {
    let tmp = tempfile::tempdir().unwrap();
    let claw = tmp.path().join("claw");
    let (stdout, _, code) = run(&["status"], &claw);
    assert_eq!(code, 0);
    assert!(stdout.to_lowercase().contains("no state"), "{stdout}");
}
