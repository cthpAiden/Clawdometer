use std::io::Read;
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

use clawdometer_core::paths;
use clawdometer_core::schema::parse_statusline_input;
use clawdometer_core::state::{now_rfc3339, render_statusline, write_state_atomic, State};
use wait_timeout::ChildExt;

const FALLBACK_LINE: &str = "clawdometer: waiting";

/// Infallible: every failure path still returns a printable line.
pub fn run_hook() -> String {
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return FALLBACK_LINE.into();
    }
    let input = match parse_statusline_input(&raw) {
        Ok(input) => input,
        Err(_) => return FALLBACK_LINE.into(),
    };
    let state = State::from_input(&input, now_rfc3339());
    // Write failure must not break the statusline; HUD just stays stale.
    let _ = write_state_atomic(&paths::state_path(), &state);
    if let Some(line) = run_wrapped(&paths::wrapped_path(), &raw) {
        return line;
    }
    render_statusline(&state)
}

/// Chain the user's original statusline command. Any failure -> None,
/// caller falls back to our own line. 2s hard timeout.
fn run_wrapped(wrapped_path: &std::path::Path, stdin_raw: &str) -> Option<String> {
    let raw = std::fs::read_to_string(wrapped_path).ok()?;
    let value: serde_json::Value =
        serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()?;
    let command = value.get("command")?.as_str()?.to_string();
    if clawdometer_core::settings::is_clawdometer_hook_command(&command) {
        // Never chain a stale clawdometer hook command into itself.
        return None;
    }

    let mut cmd = shell_command(&command);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Ignore write errors: the child may exit without reading stdin.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_raw.as_bytes());
    }
    match child.wait_timeout(Duration::from_secs(2)) {
        Ok(Some(status)) if status.success() => {
            let mut out = String::new();
            child.stdout.take()?.read_to_string(&mut out).ok()?;
            let line = out.lines().next()?.trim().to_string();
            if line.is_empty() { None } else { Some(line) }
        }
        Ok(Some(_)) => None,
        _ => {
            // On Windows, Command inherits handles (bInheritHandles=TRUE), so our
            // own stdout pipe leaks into cmd.exe and any grandchild it spawns
            // (e.g. ping.exe). child.kill() only kills cmd.exe; the grandchild
            // can survive holding that inherited handle open, so the caller
            // never sees EOF. Kill the whole process tree first.
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new("cmd");
    // raw_arg: hand the command string to cmd.exe unmangled.
    cmd.arg("/C").raw_arg(command);
    cmd
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}
