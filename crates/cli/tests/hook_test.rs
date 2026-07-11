use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const PRE: &str = include_str!("../../core/tests/fixtures/stdin-pre-response.json");
const FULL: &str = include_str!("../../core/tests/fixtures/stdin-with-limits.json");

fn run_hook(stdin: &str, clawdometer_dir: &Path) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_clawdometer"))
        .arg("hook")
        .env("CLAWDOMETER_DIR", clawdometer_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (String::from_utf8_lossy(&out.stdout).trim().to_string(), out.status.code().unwrap())
}

#[test]
fn hook_writes_state_and_prints_line() {
    let dir = tempfile::tempdir().unwrap();
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%");
    let state = clawdometer_core::state::read_state(&dir.path().join("state.json")).unwrap();
    assert_eq!(state.schema_version, 1);
    assert_eq!(state.rate_limits.unwrap().seven_day.unwrap().resets_at, 1784170800);
}

#[test]
fn hook_pre_response_writes_null_limits() {
    let dir = tempfile::tempdir().unwrap();
    let (line, code) = run_hook(PRE, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] limits pending");
    let state = clawdometer_core::state::read_state(&dir.path().join("state.json")).unwrap();
    assert!(state.rate_limits.is_none());
}
