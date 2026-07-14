//! Background usage refresher. The statusline hook only fires inside Claude
//! Code sessions that render a statusline, so usage made elsewhere (claude.ai
//! web/mobile, the desktop app) never reaches state.json. When the merged
//! snapshot goes stale, this module runs the OFFICIAL Claude Code CLI
//! headlessly — `claude -p /usage` — parses the plain-text report it prints,
//! and writes the percentages to ~/.clawdometer/live.json. The state watcher
//! sees the file change and re-emits to the HUD.
//!
//! Compliance note: Clawdometer itself still makes zero network requests and
//! never touches credentials. Claude Code fetches the numbers with its own
//! sign-in, exactly as if the user had typed `/usage`; this module only reads
//! the text output of that official, documented headless mode.

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::Duration;

use clawdometer_core::schema::{LimitWindow, RateLimits};
use clawdometer_core::state::{merge, now_rfc3339, read_state, write_state_atomic, State,
    SCHEMA_VERSION};
use wait_timeout::ChildExt;

/// Refresh when the merged snapshot is older than this — inside an active
/// statusline-fed session the data is seconds old and no refresh ever runs.
const STALE_AFTER: Duration = Duration::from_secs(60);
/// Minimum spacing between refresh attempts: one per wake-up tick. The CLI
/// is a Node process, but a single short-lived spawn per minute is cheap and
/// only happens while no statusline session is feeding fresher data.
const MIN_SPACING: Duration = Duration::from_secs(60);
/// Spacing after a failed attempt (claude not installed, parse mismatch) —
/// retrying every minute won't change anything.
const FAILURE_SPACING: Duration = Duration::from_secs(10 * 60);
/// `claude` is a Node CLI with a slow cold start; well past that is a hang.
const CLI_TIMEOUT: Duration = Duration::from_secs(90);
/// `/usage`'s "Current session/week" summary comes from a network fetch that
/// intermittently fails (~1 in 3 runs). When it does, claude still prints the
/// local analysis but omits those lines, so the parse yields nothing. Retry a
/// few times within one refresh so a single flaky fetch doesn't drop the whole
/// attempt into FAILURE_SPACING.
const REFRESH_TRIES: usize = 3;
/// Pause between retries. Measured: back-to-back `claude` runs a couple of
/// seconds apart fail reliably (exit 1 / empty output — some internal
/// concurrency or rate guard), while ~10s spacing succeeds. Anything shorter
/// makes the retries hit the very flake they exist to ride out.
const RETRY_SPACING: Duration = Duration::from_secs(10);

static REFRESH_TX: OnceLock<Sender<()>> = OnceLock::new();

/// A GUI-subsystem process starts with no console, and `claude -p /usage` only
/// prints its rate-limit summary lines when a console is attached (measured:
/// 0/5 headless runs emit them vs ~2/3 with a console). Allocate one console
/// for this process and hide its window; the /usage child then inherits it
/// (see `run_usage_command`, which no longer sets CREATE_NO_WINDOW) and prints
/// the lines, with nothing ever visible on screen. Must run on the HUD path
/// only — never the `hook` subcommand, whose stdout Claude Code captures.
#[cfg(windows)]
pub fn ensure_hidden_console() {
    use windows_sys::Win32::System::Console::{AllocConsole, GetConsoleWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    unsafe {
        if AllocConsole() != 0 {
            let hwnd = GetConsoleWindow();
            if !hwnd.is_null() {
                ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn ensure_hidden_console() {}

/// Tray "Refresh usage": skip the schedule and refresh on the next tick.
pub fn request_refresh() {
    if let Some(tx) = REFRESH_TX.get() {
        let _ = tx.send(());
    }
}

pub fn spawn() {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let _ = REFRESH_TX.set(tx);
    std::thread::spawn(move || {
        let mut last_attempt: Option<std::time::Instant> = None;
        let mut last_failed = false;
        loop {
            // Wake on an explicit request or every minute to check staleness.
            let explicit = match rx.recv_timeout(Duration::from_secs(60)) {
                Ok(()) => true,
                Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let spacing = if last_failed { FAILURE_SPACING } else { MIN_SPACING };
            let spaced_out =
                last_attempt.is_some_and(|t| t.elapsed() < spacing) && !explicit;
            if spaced_out || (!explicit && !snapshot_is_stale()) {
                continue;
            }
            last_attempt = Some(std::time::Instant::now());
            last_failed = !refresh_once();
            // A refresh can take minutes (CLI timeouts × retries). Tray clicks
            // queued while it ran — or spammed before it — are satisfied by
            // the refresh that just finished; drain them so each doesn't spawn
            // another back-to-back `claude` run.
            while rx.try_recv().is_ok() {}
        }
    });
}

/// Newest of state.json/live.json older than STALE_AFTER (or absent/unparsable).
fn snapshot_is_stale() -> bool {
    let merged = merge(
        read_state(&clawdometer_core::paths::state_path()),
        read_state(&clawdometer_core::paths::live_path()),
    );
    let Some(s) = merged else { return true };
    let Ok(t) = time::OffsetDateTime::parse(
        &s.captured_at,
        &time::format_description::well_known::Rfc3339,
    ) else {
        return true;
    };
    let age = time::OffsetDateTime::now_utc() - t;
    // Future-stamped captured_at (clock stepped back) is as untrustworthy as
    // old data — the UI already flags it stale, so refresh it too instead of
    // treating it as forever-fresh. Small negative slack tolerates sub-minute
    // clock differences.
    age > STALE_AFTER || age < -time::Duration::seconds(60)
}

fn refresh_once() -> bool {
    for attempt in 0..REFRESH_TRIES {
        if attempt > 0 {
            std::thread::sleep(RETRY_SPACING);
        }
        let Some(text) = run_usage_command() else { continue };
        let now = time::OffsetDateTime::now_utc();
        let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
        let Some(rate_limits) = parse_usage_text(&text, now, offset) else { continue };
        let state = State {
            schema_version: SCHEMA_VERSION,
            captured_at: now_rfc3339(),
            rate_limits: Some(rate_limits),
            model: None,
            context_window: None,
            session_id: None,
            transcript_path: None,
            cli_version: None,
        };
        return write_state_atomic(&clawdometer_core::paths::live_path(), &state).is_ok();
    }
    false
}

/// Run `claude -p /usage` headlessly and return its stdout. `claude` on
/// Windows is an npm .cmd shim, so it goes through cmd.exe (fixed command
/// string — nothing user-controlled is interpolated).
/// --no-session-persistence: without it every run dumps a ~20 KB session
/// transcript into ~/.claude/projects/ — 1440 files a day at this cadence.
fn run_usage_command() -> Option<String> {
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new(clawdometer_core::paths::system32_exe("cmd.exe"));
        c.arg("/C").raw_arg("claude -p --no-session-persistence /usage");
        // Deliberately no CREATE_NO_WINDOW: the child must inherit this
        // process's console (ensure_hidden_console) so claude prints the
        // rate-limit summary. That console's window is hidden, so inheriting it
        // flashes nothing — whereas CREATE_NO_WINDOW detaches the console and
        // claude silently drops the numbers.
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("claude");
        c.args(["-p", "--no-session-persistence", "/usage"]);
        c
    };
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Drain stdout on a thread (same pattern as the hook's run_wrapped): a
    // pipe fills at ~64 KiB, and /usage output plus a hung reader would
    // deadlock a plain wait.
    let (tx, rx) = std::sync::mpsc::channel();
    if let Some(mut stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
        });
    }
    match child.wait_timeout(CLI_TIMEOUT) {
        Ok(Some(status)) if status.success() => rx.recv_timeout(Duration::from_secs(5)).ok(),
        _ => {
            // child.kill() would only kill cmd.exe — the claude grandchild
            // survives and, at one attempt per minute, hung ones pile up.
            clawdometer_core::hook::kill_tree(&mut child);
            None
        }
    }
}

/// Parse the `/usage` plain-text report. Expected lines (v2.1, 2026-07):
///   Current session: 32% used · resets Jul 13, 3:29pm (Asia/Saigon)
///   Current week (all models): 39% used · resets Jul 16, 9:59am (Asia/Saigon)
/// Session maps to five_hour, week (all models) to seven_day. Any line that
/// doesn't parse is skipped; None when neither window parsed (e.g. the CLI
/// changed its wording) — the caller then just keeps the old snapshot.
fn parse_usage_text(
    text: &str,
    now: time::OffsetDateTime,
    local_offset: time::UtcOffset,
) -> Option<RateLimits> {
    let window_from = |prefix: &str| -> Option<LimitWindow> {
        let line = text.lines().find(|l| l.trim_start().starts_with(prefix))?;
        let rest = line.split_once(": ")?.1;
        let pct: i64 = rest.split('%').next()?.trim().parse().ok()?;
        let reset_str = rest.split_once("resets ")?.1;
        let reset_str = reset_str.split(" (").next()?.trim();
        let resets_at = parse_reset(reset_str, now, local_offset)?;
        Some(LimitWindow { used_percentage: pct, resets_at })
    };
    let five_hour = window_from("Current session:");
    let seven_day = window_from("Current week (all models):");
    if five_hour.is_none() && seven_day.is_none() {
        return None;
    }
    Some(RateLimits { five_hour, seven_day })
}

/// "Jul 13, 3:29pm" or "Jul 16, 10am" (printed in the machine's local
/// timezone, no year; ":00" minutes are dropped) -> epoch seconds. The year
/// is inferred: resets are at most 7 days ahead, so a parse landing far in
/// the past means the year rolled over (Dec -> Jan).
fn parse_reset(
    s: &str,
    now: time::OffsetDateTime,
    local_offset: time::UtcOffset,
) -> Option<i64> {
    // "10am" -> "10:00am": no month abbreviation contains a lowercase
    // "am"/"pm", so the replacement can only hit the time-of-day.
    let s = if s.contains(':') {
        s.to_owned()
    } else {
        s.replace("am", ":00am").replace("pm", ":00pm")
    };
    let fmt = time::format_description::parse_borrowed::<2>(
        "[year] [month repr:short] [day padding:none], \
         [hour repr:12 padding:none]:[minute][period case:lower]",
    )
    .ok()?;
    let year = now.to_offset(local_offset).year();
    for candidate_year in [year, year + 1] {
        let Ok(dt) =
            time::PrimitiveDateTime::parse(&format!("{candidate_year} {s}"), &fmt)
        else {
            return None;
        };
        let epoch = dt.assume_offset(local_offset).unix_timestamp();
        // More than a day in the past can't be a reset schedule — try the
        // next year (Dec 31 -> Jan rollover).
        if epoch >= (now - Duration::from_secs(24 * 3600)).unix_timestamp() {
            return Some(epoch);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured verbatim from `claude -p /usage` (v2.1.207, 2026-07-13);
    // attribution sections trimmed.
    const REPORT: &str = "\
You are currently using your subscription to power your Claude Code usage

Current session: 32% used \u{b7} resets Jul 13, 3:29pm (Asia/Saigon)
Current week (all models): 39% used \u{b7} resets Jul 16, 9:59am (Asia/Saigon)
Current week (Fable): 54% used \u{b7} resets Jul 16, 9:59am (Asia/Saigon)

What's contributing to your limits usage?
";

    fn t(rfc3339: &str) -> time::OffsetDateTime {
        time::OffsetDateTime::parse(rfc3339, &time::format_description::well_known::Rfc3339)
            .unwrap()
    }

    #[test]
    fn parses_real_usage_report() {
        // Machine tz was UTC+7 (Asia/Saigon) when the report was captured.
        let offset = time::UtcOffset::from_hms(7, 0, 0).unwrap();
        let rl = parse_usage_text(REPORT, t("2026-07-13T06:00:00Z"), offset).unwrap();
        let fh = rl.five_hour.unwrap();
        assert_eq!(fh.used_percentage, 32);
        // Jul 13 3:29pm +07:00 == 2026-07-13T08:29:00Z
        assert_eq!(fh.resets_at, t("2026-07-13T08:29:00Z").unix_timestamp());
        let sd = rl.seven_day.unwrap();
        assert_eq!(sd.used_percentage, 39);
        assert_eq!(sd.resets_at, t("2026-07-16T02:59:00Z").unix_timestamp());
    }

    #[test]
    fn ignores_the_per_model_week_line() {
        let offset = time::UtcOffset::from_hms(7, 0, 0).unwrap();
        let rl = parse_usage_text(REPORT, t("2026-07-13T06:00:00Z"), offset).unwrap();
        assert_ne!(rl.seven_day.unwrap().used_percentage, 54, "must not read the Fable line");
    }

    #[test]
    fn rejects_reworded_output() {
        let offset = time::UtcOffset::UTC;
        assert!(parse_usage_text("Usage: everything is different now", t("2026-07-13T06:00:00Z"), offset).is_none());
        assert!(parse_usage_text("", t("2026-07-13T06:00:00Z"), offset).is_none());
    }

    #[test]
    fn one_parsable_window_is_enough() {
        let offset = time::UtcOffset::UTC;
        let text = "Current session: 5% used \u{b7} resets Jul 13, 3:29pm (UTC)\n";
        let rl = parse_usage_text(text, t("2026-07-13T06:00:00Z"), offset).unwrap();
        assert_eq!(rl.five_hour.unwrap().used_percentage, 5);
        assert!(rl.seven_day.is_none());
    }

    #[test]
    fn parses_week_line_with_minuteless_reset_time() {
        // Captured 2026-07-13: when minutes are :00 the CLI prints "10am",
        // not "10:00am" — this silently dropped the weekly window.
        let offset = time::UtcOffset::from_hms(7, 0, 0).unwrap();
        let text =
            "Current week (all models): 40% used \u{b7} resets Jul 16, 10am (Asia/Saigon)\n";
        let rl = parse_usage_text(text, t("2026-07-13T06:00:00Z"), offset).unwrap();
        let sd = rl.seven_day.unwrap();
        assert_eq!(sd.used_percentage, 40);
        // Jul 16 10:00am +07:00 == 2026-07-16T03:00:00Z
        assert_eq!(sd.resets_at, t("2026-07-16T03:00:00Z").unix_timestamp());
    }

    #[test]
    fn reset_year_rolls_over_in_january() {
        // Dec 31 local time, report says "resets Jan 2, 1:00am" — must land in
        // the NEXT year, not eleven+ months in the past.
        let offset = time::UtcOffset::UTC;
        let now = t("2026-12-31T20:00:00Z");
        let epoch = parse_reset("Jan 2, 1:00am", now, offset).unwrap();
        assert_eq!(epoch, t("2027-01-02T01:00:00Z").unix_timestamp());
    }

    #[test]
    fn reset_parses_padded_and_unpadded_days_and_hours() {
        let offset = time::UtcOffset::UTC;
        let now = t("2026-07-13T06:00:00Z");
        assert_eq!(
            parse_reset("Jul 14, 9:05am", now, offset).unwrap(),
            t("2026-07-14T09:05:00Z").unix_timestamp()
        );
        assert_eq!(
            parse_reset("Jul 14, 12:00am", now, offset).unwrap(),
            t("2026-07-14T00:00:00Z").unix_timestamp()
        );
        assert_eq!(
            parse_reset("Jul 14, 12:00pm", now, offset).unwrap(),
            t("2026-07-14T12:00:00Z").unix_timestamp()
        );
        // minuteless variants ("10am", "12pm") — the CLI drops ":00"
        assert_eq!(
            parse_reset("Jul 16, 10am", now, offset).unwrap(),
            t("2026-07-16T10:00:00Z").unix_timestamp()
        );
        assert_eq!(
            parse_reset("Jul 14, 12pm", now, offset).unwrap(),
            t("2026-07-14T12:00:00Z").unix_timestamp()
        );
        assert_eq!(
            parse_reset("Jul 14, 12am", now, offset).unwrap(),
            t("2026-07-14T00:00:00Z").unix_timestamp()
        );
    }

    #[test]
    fn garbage_reset_is_rejected() {
        let offset = time::UtcOffset::UTC;
        let now = t("2026-07-13T06:00:00Z");
        assert!(parse_reset("tomorrow-ish", now, offset).is_none());
        assert!(parse_reset("", now, offset).is_none());
    }
}
