//! Full lifecycle against a sandboxed settings.json:
//! install -> run the EXACT installed command string with fixture stdin ->
//! verify state.json -> uninstall -> deep-equal restore.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const PRE: &str = include_str!("../../core/tests/fixtures/stdin-pre-response.json");
const FULL: &str = include_str!("../../core/tests/fixtures/stdin-with-limits.json");

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(
        std::fs::read_to_string(path).unwrap().trim_start_matches('\u{feff}'),
    ).unwrap()
}

#[test]
fn full_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("claw");
    let original = r#"{"model":"opus","statusLine":{"command":"echo pre-existing","padding":1}}"#;
    std::fs::write(&settings, original).unwrap();

    // 1. install
    let status = Command::new(env!("CARGO_BIN_EXE_clawdometer"))
        .args(["install", "--settings", settings.to_str().unwrap()])
        .env("CLAWDOMETER_DIR", &claw)
        .status()
        .unwrap();
    assert!(status.success());

    // 2. run the EXACT command Claude Code would run (from settings.json, via cmd /C)
    let installed_cmd = read_json(&settings)["statusLine"]["command"]
        .as_str().unwrap().to_string();
    for (fixture, expect_limits) in [(PRE, false), (FULL, true)] {
        let mut child = shell(&installed_cmd)
            .env("CLAWDOMETER_DIR", &claw)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(fixture.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "installed command must exit 0");
        // wrapped pre-existing statusline is chained through
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "pre-existing");
        let state = clawdometer_core::state::read_state(&claw.join("state.json")).unwrap();
        assert_eq!(state.rate_limits.is_some(), expect_limits);
    }

    // 3. uninstall restores original deep-equal
    let status = Command::new(env!("CARGO_BIN_EXE_clawdometer"))
        .args(["uninstall", "--settings", settings.to_str().unwrap()])
        .env("CLAWDOMETER_DIR", &claw)
        .status()
        .unwrap();
    assert!(status.success());
    let before: serde_json::Value = serde_json::from_str(original).unwrap();
    assert_eq!(read_json(&settings), before);
}

#[cfg(windows)]
fn shell(command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").raw_arg(command);
    cmd
}

#[cfg(not(windows))]
fn shell(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}
