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
    disable_stdio_inheritance();
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
    // Write stdin on a detached thread. A child that never reads stdin would
    // otherwise block write_all once the payload exceeds the pipe buffer —
    // before wait_timeout below ever starts. Write errors are ignored: the
    // child may exit without reading stdin.
    if let Some(mut stdin) = child.stdin.take() {
        let raw = stdin_raw.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(raw.as_bytes());
        });
    }

    // Drain stdout concurrently on a thread, started BEFORE we wait on the
    // child. A grandchild can hold the inherited stdout handle open (see
    // kill_tree below) even after the child itself has exited, so a plain
    // read_to_string after wait_timeout can block forever; reading on a
    // separate thread lets us bound how long we wait for output instead.
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(mut stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut out = String::new();
            let _ = stdout.read_to_string(&mut out);
            let _ = tx.send(out);
        });
    } else {
        let _ = tx.send(String::new());
    }

    match child.wait_timeout(Duration::from_secs(2)) {
        Ok(Some(status)) if status.success() => {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(out) => {
                    let line = out.lines().next()?.trim().to_string();
                    if line.is_empty() { None } else { Some(line) }
                }
                Err(_) => {
                    // stdout never closed — a lingering grandchild is still
                    // holding the pipe open. Give up and reap the tree.
                    kill_tree(&mut child);
                    None
                }
            }
        }
        Ok(Some(_)) => None,
        _ => {
            kill_tree(&mut child);
            None
        }
    }
}

/// On Windows, Command inherits handles (bInheritHandles=TRUE), so our own
/// stdout pipe leaks into cmd.exe and any grandchild it spawns (e.g.
/// ping.exe). child.kill() only kills cmd.exe; the grandchild can survive
/// holding that inherited handle open, so the caller never sees EOF. Kill
/// the whole process tree first.
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = Command::new(paths::system32_exe("taskkill.exe"))
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// On Windows, Rust's `Command` spawns children with `bInheritHandles=TRUE`,
/// which inherits *every* inheritable handle currently open in this process —
/// not just the ones explicitly wired up as stdio — into any child (and its
/// grandchildren) we spawn. If our own stdout/stdin were inherited from
/// whoever launched us (e.g. Claude Code, or a test harness), a lingering
/// grandchild of the wrapped command (see kill_tree) could hold a duplicate
/// of *our* stdout open, blocking whoever is reading from us long after we
/// exit. Clear the inherit flag on our own std handles before spawning
/// anything, so only the pipes we explicitly create for the child propagate.
#[cfg(windows)]
fn disable_stdio_inheritance() {
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    unsafe {
        SetHandleInformation(GetStdHandle(STD_OUTPUT_HANDLE), HANDLE_FLAG_INHERIT, 0);
        SetHandleInformation(GetStdHandle(STD_INPUT_HANDLE), HANDLE_FLAG_INHERIT, 0);
        SetHandleInformation(GetStdHandle(STD_ERROR_HANDLE), HANDLE_FLAG_INHERIT, 0);
    }
}

#[cfg(not(windows))]
fn disable_stdio_inheritance() {}

#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new(paths::system32_exe("cmd.exe"));
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
