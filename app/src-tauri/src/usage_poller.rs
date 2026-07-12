//! Live usage poller. The statusline hook only fires while a Claude Code
//! session is actively making API calls, so on its own the HUD goes stale the
//! moment the user stops. This poller queries Anthropic's OAuth usage
//! endpoint (the same source `/usage` reads) every 60s with the access token
//! Claude Code already stores in ~/.claude/.credentials.json, and writes the
//! result to ~/.clawdometer/live.json. The state watcher sees the file change
//! and re-emits to the HUD — no extra IPC.
//!
//! The HTTP call shells out to curl.exe (ships with Windows 10 1803+) so the
//! workspace keeps its ban on network crates: the hook/CLI remain provably
//! network-free, and the only outbound request in the whole app is this one
//! read-only GET to api.anthropic.com.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

use clawdometer_core::schema::{LimitWindow, RateLimits};
use clawdometer_core::state::{now_rfc3339, write_state_atomic, State, SCHEMA_VERSION};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const POLL_INTERVAL: Duration = Duration::from_secs(60);

pub fn spawn() {
    std::thread::spawn(|| loop {
        if let Some(rate_limits) = poll_once() {
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
            let _ = write_state_atomic(&clawdometer_core::paths::live_path(), &state);
        }
        std::thread::sleep(POLL_INTERVAL);
    });
}

fn poll_once() -> Option<RateLimits> {
    let raw =
        std::fs::read_to_string(clawdometer_core::paths::claude_credentials_path()).ok()?;
    let token = access_token(&raw)?;
    let body = fetch(&token)?;
    parse_usage(&body)
}

fn access_token(credentials_json: &str) -> Option<String> {
    let v: serde_json::Value =
        serde_json::from_str(credentials_json.trim_start_matches('\u{feff}')).ok()?;
    let token = v.pointer("/claudeAiOauth/accessToken")?.as_str()?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// GET the usage endpoint. The token is passed to curl as a `--config -`
/// header line on stdin, never on argv (argv is visible to every process on
/// the machine).
fn fetch(token: &str) -> Option<String> {
    let mut cmd = Command::new(clawdometer_core::paths::system32_exe("curl.exe"));
    cmd.args(["--silent", "--fail", "--max-time", "15", "--config", "-", USAGE_URL]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let config = format!(
        "header = \"Authorization: Bearer {token}\"\nheader = \"anthropic-beta: oauth-2025-04-20\"\n"
    );
    // Ignore write errors (curl can exit before reading its config); the
    // early `?` here used to skip wait_with_output, leaving the child unreaped.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(config.as_bytes());
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Maps the endpoint's `{five_hour: {utilization, resets_at}, seven_day: …}`
/// (float percent, RFC3339 reset time) onto the statusline's LimitWindow
/// shape (integer percent, epoch seconds).
fn parse_usage(body: &str) -> Option<RateLimits> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let window = |key: &str| -> Option<LimitWindow> {
        let w = v.get(key)?;
        let pct = w.get("utilization")?.as_f64()?;
        let resets = w.get("resets_at")?.as_str()?;
        let t = time::OffsetDateTime::parse(
            resets,
            &time::format_description::well_known::Rfc3339,
        )
        .ok()?;
        Some(LimitWindow {
            used_percentage: pct.round() as i64,
            resets_at: t.unix_timestamp(),
        })
    };
    let five_hour = window("five_hour");
    let seven_day = window("seven_day");
    if five_hour.is_none() && seven_day.is_none() {
        return None;
    }
    Some(RateLimits { five_hour, seven_day })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from a real response (2026-07-12); unused fields dropped.
    const BODY: &str = r#"{
        "five_hour": {"utilization": 4.0, "resets_at": "2026-07-12T04:59:59.702684+00:00", "limit_dollars": null},
        "seven_day": {"utilization": 16.4, "resets_at": "2026-07-16T02:59:59.702707+00:00"},
        "seven_day_opus": null
    }"#;

    #[test]
    fn parses_real_usage_body() {
        let rl = parse_usage(BODY).unwrap();
        let fh = rl.five_hour.unwrap();
        assert_eq!(fh.used_percentage, 4);
        assert_eq!(fh.resets_at, 1783832399); // 2026-07-12T04:59:59Z
        assert_eq!(rl.seven_day.unwrap().used_percentage, 16);
    }

    #[test]
    fn rejects_garbage_and_empty() {
        assert!(parse_usage("not json").is_none());
        assert!(parse_usage("{}").is_none());
        assert!(parse_usage(r#"{"five_hour": {"utilization": null}}"#).is_none());
    }

    #[test]
    fn extracts_access_token() {
        assert_eq!(
            access_token(r#"{"claudeAiOauth": {"accessToken": "tok-123"}}"#).as_deref(),
            Some("tok-123")
        );
        assert!(access_token(r#"{"claudeAiOauth": {"accessToken": ""}}"#).is_none());
        assert!(access_token("{}").is_none());
        assert!(access_token("garbage").is_none());
    }
}
