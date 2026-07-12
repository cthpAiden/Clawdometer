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

#[test]
fn hook_survives_garbage_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let (line, code) = run_hook("%%% not json {{{", dir.path());
    assert_eq!(code, 0);
    assert!(!line.is_empty());
    assert!(!dir.path().join("state.json").exists(), "garbage must not produce a state file");
}

#[test]
fn hook_survives_empty_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let (line, code) = run_hook("", dir.path());
    assert_eq!(code, 0);
    assert!(!line.is_empty());
}

#[test]
fn hook_survives_unwritable_state_dir() {
    // CLAWDOMETER_DIR whose parent is a FILE -> create_dir_all fails.
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "i am a file").unwrap();
    let bad_dir = blocker.join("nested");
    let (line, code) = run_hook(FULL, &bad_dir);
    assert_eq!(code, 0, "unwritable dir must still exit 0");
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%", "statusline still renders from parsed input");
}

fn write_wrapped(dir: &Path, command: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let obj = serde_json::json!({ "command": command, "padding": 0 });
    std::fs::write(dir.join("wrapped.json"), serde_json::to_string(&obj).unwrap()).unwrap();
}

#[test]
fn hook_passes_through_wrapped_command_output() {
    let dir = tempfile::tempdir().unwrap();
    write_wrapped(dir.path(), "echo original-statusline");
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "original-statusline");
    // state.json still written even when passing through
    assert!(clawdometer_core::state::read_state(&dir.path().join("state.json")).is_some());
}

#[test]
fn hook_falls_back_when_wrapped_command_fails() {
    let dir = tempfile::tempdir().unwrap();
    write_wrapped(dir.path(), "cmd /C exit 3");
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%");
}

#[test]
fn hook_falls_back_when_wrapped_command_hangs() {
    let dir = tempfile::tempdir().unwrap();
    // ping -n 30 sleeps ~29s on Windows; must be killed at the 2s timeout.
    write_wrapped(dir.path(), "ping -n 30 127.0.0.1");
    let start = std::time::Instant::now();
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%");
    assert!(start.elapsed() < std::time::Duration::from_secs(10), "timeout did not fire");
}

#[test]
fn hook_never_chains_a_stale_clawdometer_hook_command() {
    let dir = tempfile::tempdir().unwrap();
    // Simulate wrapped.json accidentally pointing at a clawdometer hook
    // invocation (e.g. a stale self-wrap from before the install-time fix).
    write_wrapped(dir.path(), r#""C:\old path\clawdometer.exe" hook"#);
    let start = std::time::Instant::now();
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%", "must fall back to our own render, not chain");
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "must not attempt to spawn/wait on the chained command");
}

#[test]
fn hook_never_chains_the_real_current_exe_self_wrap() {
    // Point wrapped.json at the ACTUAL current exe with `hook` — this is the
    // true self-wrap scenario: without the guard, run_wrapped would spawn a
    // real clawdometer process that recurses into itself indefinitely
    // (each level spawning another, sharing the same CLAWDOMETER_DIR/wrapped.json).
    let dir = tempfile::tempdir().unwrap();
    let exe = env!("CARGO_BIN_EXE_clawdometer");
    write_wrapped(dir.path(), &format!("\"{exe}\" hook"));
    let start = std::time::Instant::now();
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%", "must fall back, never chain into itself");
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "self-wrap recursion must be short-circuited, not merely timed out");
}

#[test]
fn hook_does_not_hang_when_wrapped_command_leaves_grandchild_holding_stdout() {
    let dir = tempfile::tempdir().unwrap();
    // The wrapped command itself exits successfully and prints a line, but
    // leaves a lingering background grandchild alive that (on Windows) still
    // holds the inherited stdout pipe handle open, so plain read_to_string
    // on that handle would never see EOF.
    write_wrapped(dir.path(), r#"cmd /C "start /B ping -n 30 127.0.0.1 & echo chained-line""#);
    let start = std::time::Instant::now();
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert!(
        line == "chained-line" || line == "[Opus 4.8 (1M context)] 5h 1% · 7d 5%",
        "expected chained output or fallback, got: {line}"
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(10), "must not hang on lingering grandchild holding stdout open");
}

#[test]
fn hook_does_not_hang_when_wrapped_command_ignores_large_stdin() {
    let dir = tempfile::tempdir().unwrap();
    // ping never reads stdin. With a payload larger than the pipe buffer, a
    // synchronous stdin write in run_wrapped would block before the 2s
    // wait_timeout ever starts, hanging the hook until ping exits (~29s).
    write_wrapped(dir.path(), "ping -n 30 127.0.0.1");
    let big = format!(
        r#"{{"model":{{"id":"x","display_name":"X"}},"pad":"{}"}}"#,
        "x".repeat(2_000_000)
    );
    let start = std::time::Instant::now();
    let (line, code) = run_hook(&big, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[X] limits pending");
    assert!(
        start.elapsed() < std::time::Duration::from_secs(10),
        "stdin write to a non-reading child must not defeat the 2s timeout"
    );
}

#[test]
fn hook_does_not_hang_when_grandchild_holds_stderr() {
    // Same class as the stdout-handle bug: whoever launched the hook may read
    // our STDERR to EOF. A lingering grandchild of the wrapped command
    // inherits our stderr handle (bInheritHandles=TRUE copies every
    // inheritable handle) and keeps it open after we exit, so the reader
    // never sees EOF unless the inherit flag is cleared on stderr too.
    let dir = tempfile::tempdir().unwrap();
    write_wrapped(dir.path(), r#"cmd /C "start /B ping -n 30 127.0.0.1 & echo chained-line""#);
    let start = std::time::Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_clawdometer"))
        .arg("hook")
        .env("CLAWDOMETER_DIR", dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped()) // piped: reader waits for stderr EOF
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(FULL.as_bytes()).unwrap();
    // wait_with_output drains stdout AND stderr to EOF before returning.
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        line == "chained-line" || line == "[Opus 4.8 (1M context)] 5h 1% · 7d 5%",
        "expected chained output or fallback, got: {line}"
    );
    assert!(
        start.elapsed() < std::time::Duration::from_secs(10),
        "grandchild holding inherited stderr must not block our reader's EOF"
    );
}

#[test]
fn hook_falls_back_when_wrapped_json_malformed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path()).unwrap();
    std::fs::write(dir.path().join("wrapped.json"), "{ nope").unwrap();
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%");
}
