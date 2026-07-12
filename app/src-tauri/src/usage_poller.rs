//! Live usage poller. The statusline hook only fires while a Claude Code
//! session is actively making API calls, so on its own the HUD goes stale the
//! moment the user stops. This poller queries Anthropic's OAuth usage
//! endpoint (the same source `/usage` reads) every 60s with the access token
//! Claude Code already stores in ~/.claude/.credentials.json, and writes the
//! result to ~/.clawdometer/live.json. The state watcher sees the file change
//! and re-emits to the HUD — no extra IPC.
//!
//! The HTTP calls shell out to curl.exe (ships with Windows 10 1803+) so the
//! workspace keeps its ban on network crates: the hook/CLI remain provably
//! network-free, and the only outbound requests in the whole app are this
//! read-only GET to api.anthropic.com plus, when the stored access token has
//! expired, one OAuth refresh POST to the same host.
//!
//! Without the refresh, the poller dies the moment the access token expires
//! (~8h): the usage endpoint starts answering 429 and live.json freezes until
//! the user opens Claude Code to refresh the token. The refresh rotates the
//! refresh token, so the updated credentials are written back to
//! ~/.claude/.credentials.json atomically — otherwise Claude Code's own copy
//! would be invalidated and the user signed out.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

use clawdometer_core::schema::{LimitWindow, RateLimits};
use clawdometer_core::state::{now_rfc3339, write_state_atomic, State, SCHEMA_VERSION};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const TOKEN_URL: &str = "https://api.anthropic.com/v1/oauth/token";
/// Claude Code's public OAuth client id (PKCE client, not a secret).
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Cloudflare answers 429 to curl's default user-agent on bad-auth requests
/// (verified 2026-07-12); any explicit UA gets real status codes.
const USER_AGENT: &str = concat!("clawdometer/", env!("CARGO_PKG_VERSION"));
const POLL_INTERVAL: Duration = Duration::from_secs(60);
/// Refresh slightly before expiry so a poll never races the deadline.
const EXPIRY_MARGIN_MS: i64 = 60_000;

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
    let cred_path = clawdometer_core::paths::claude_credentials_path();
    let raw = std::fs::read_to_string(&cred_path).ok()?;
    let mut creds: serde_json::Value =
        serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()?;
    let now_ms = unix_now_ms();
    if token_expired(&creds, now_ms) {
        if let Some(fresh) = refresh(&creds, now_ms) {
            // Re-read + merge is unnecessary: Claude Code rewrites the whole
            // file on its own refreshes, and we only refresh when the token
            // is already expired (so Claude Code is not mid-session).
            let _ = write_credentials_atomic(&cred_path, &fresh);
            creds = fresh;
        }
        // Refresh failure falls through to try the stored token anyway —
        // expiresAt could be wrong, and a failed GET costs nothing extra.
    }
    let token = access_token(&creds)?;
    let body = fetch(&token)?;
    parse_usage(&body)
}

fn unix_now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// Expired (or about to) per the stored `expiresAt` (epoch millis). Missing
/// or malformed expiry means "not expired": try the token as-is.
fn token_expired(creds: &serde_json::Value, now_ms: i64) -> bool {
    creds
        .pointer("/claudeAiOauth/expiresAt")
        .and_then(|v| v.as_i64())
        .is_some_and(|at| at <= now_ms + EXPIRY_MARGIN_MS)
}

fn access_token(creds: &serde_json::Value) -> Option<String> {
    let token = creds.pointer("/claudeAiOauth/accessToken")?.as_str()?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// POST the refresh grant and fold the response into a copy of the
/// credentials. None on any failure (caller keeps the stored token).
fn refresh(creds: &serde_json::Value, now_ms: i64) -> Option<serde_json::Value> {
    let refresh_token = creds.pointer("/claudeAiOauth/refreshToken")?.as_str()?;
    if refresh_token.is_empty() {
        return None;
    }
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
    })
    .to_string();
    let response = post_token(&body)?;
    apply_refresh(creds, &response, now_ms)
}

/// Merge a token-endpoint response (`access_token`, `refresh_token`,
/// `expires_in` seconds, `refresh_token_expires_in` seconds) into the
/// credentials JSON, preserving every field it doesn't own.
fn apply_refresh(
    creds: &serde_json::Value,
    response_body: &str,
    now_ms: i64,
) -> Option<serde_json::Value> {
    let resp: serde_json::Value = serde_json::from_str(response_body).ok()?;
    let access = resp.get("access_token")?.as_str()?;
    if access.is_empty() {
        return None;
    }
    let mut out = creds.clone();
    let oauth = out.pointer_mut("/claudeAiOauth")?.as_object_mut()?;
    oauth.insert("accessToken".into(), access.into());
    if let Some(rt) = resp.get("refresh_token").and_then(|v| v.as_str()) {
        if !rt.is_empty() {
            oauth.insert("refreshToken".into(), rt.into());
        }
    }
    if let Some(secs) = resp.get("expires_in").and_then(|v| v.as_i64()) {
        oauth.insert("expiresAt".into(), (now_ms + secs * 1000).into());
    }
    if let Some(secs) = resp.get("refresh_token_expires_in").and_then(|v| v.as_i64()) {
        oauth.insert("refreshTokenExpiresAt".into(), (now_ms + secs * 1000).into());
    }
    Some(out)
}

/// POST `body` (contains the refresh token, so it goes over stdin via
/// `--data @-`, never argv) to the token endpoint.
fn post_token(body: &str) -> Option<String> {
    let mut cmd = Command::new(clawdometer_core::paths::system32_exe("curl.exe"));
    cmd.args([
        "--silent",
        "--fail",
        "--max-time",
        "15",
        "-A",
        USER_AGENT,
        "-H",
        "Content-Type: application/json",
        "--data",
        "@-",
        TOKEN_URL,
    ]);
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
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.as_bytes());
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Same temp-file-in-dir + rename dance as write_state_atomic: Claude Code
/// must never see a torn credentials file.
fn write_credentials_atomic(
    path: &std::path::Path,
    creds: &serde_json::Value,
) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "credentials path has no parent")
    })?;
    let body = serde_json::to_string(creds)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(body.as_bytes())?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
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

    fn creds(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn extracts_access_token() {
        assert_eq!(
            access_token(&creds(r#"{"claudeAiOauth": {"accessToken": "tok-123"}}"#)).as_deref(),
            Some("tok-123")
        );
        assert!(access_token(&creds(r#"{"claudeAiOauth": {"accessToken": ""}}"#)).is_none());
        assert!(access_token(&creds("{}")).is_none());
    }

    #[test]
    fn expiry_check_uses_margin_and_tolerates_missing_field() {
        let c = creds(r#"{"claudeAiOauth": {"expiresAt": 1000000}}"#);
        assert!(token_expired(&c, 1_000_000)); // at expiry
        assert!(token_expired(&c, 1_000_000 - EXPIRY_MARGIN_MS)); // inside margin
        assert!(!token_expired(&c, 1_000_000 - EXPIRY_MARGIN_MS - 1)); // outside
        assert!(!token_expired(&creds("{}"), 1_000_000)); // no field: try as-is
        assert!(!token_expired(&creds(r#"{"claudeAiOauth": {"expiresAt": "soon"}}"#), 1_000_000));
    }

    #[test]
    fn apply_refresh_rotates_tokens_and_preserves_other_fields() {
        let c = creds(
            r#"{"claudeAiOauth": {"accessToken": "old-a", "refreshToken": "old-r",
                "expiresAt": 1, "scopes": ["user:profile"], "subscriptionType": "max"}}"#,
        );
        // Shape from a real 2026-07-12 response (unused fields dropped).
        let resp = r#"{"access_token": "new-a", "refresh_token": "new-r",
            "expires_in": 28800, "refresh_token_expires_in": 777600, "token_type": "Bearer"}"#;
        let out = apply_refresh(&c, resp, 1_000_000).unwrap();
        let o = out.pointer("/claudeAiOauth").unwrap();
        assert_eq!(o["accessToken"], "new-a");
        assert_eq!(o["refreshToken"], "new-r");
        assert_eq!(o["expiresAt"], 1_000_000 + 28_800_000);
        assert_eq!(o["refreshTokenExpiresAt"], 1_000_000 + 777_600_000);
        assert_eq!(o["scopes"][0], "user:profile"); // untouched fields survive
        assert_eq!(o["subscriptionType"], "max");
    }

    #[test]
    fn apply_refresh_keeps_old_refresh_token_when_none_returned() {
        let c = creds(r#"{"claudeAiOauth": {"accessToken": "old-a", "refreshToken": "old-r"}}"#);
        let out =
            apply_refresh(&c, r#"{"access_token": "new-a", "expires_in": 60}"#, 0).unwrap();
        assert_eq!(out.pointer("/claudeAiOauth/refreshToken").unwrap(), "old-r");
    }

    #[test]
    fn apply_refresh_rejects_bad_responses() {
        let c = creds(r#"{"claudeAiOauth": {"accessToken": "old-a"}}"#);
        assert!(apply_refresh(&c, "not json", 0).is_none());
        assert!(apply_refresh(&c, "{}", 0).is_none());
        assert!(apply_refresh(&c, r#"{"access_token": ""}"#, 0).is_none());
    }
}
