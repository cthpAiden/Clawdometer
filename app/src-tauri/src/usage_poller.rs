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
const MAX_BACKOFF: Duration = Duration::from_secs(1800);
/// Refresh slightly before expiry so a poll never races the deadline.
const EXPIRY_MARGIN_MS: i64 = 60_000;

/// Outcome of one poll cycle: drives backoff and the HUD's stale-data hint.
enum PollOutcome {
    Success(RateLimits),
    /// Token rejected by the server — opening Claude Code fixes it.
    Auth,
    /// No usable credentials on disk (file missing, unreadable, or without an
    /// OAuth token) — installing/signing in to Claude Code fixes it.
    NoCredentials,
    /// System32 curl.exe is absent (Windows 10 before 1803) — the poller has
    /// no transport at all.
    NoCurl,
    /// Network down, 429/5xx, or malformed response — retrying fixes it.
    Transient,
}

/// poll_error.json kind for a failed cycle; None for success.
fn error_kind(outcome: &PollOutcome) -> Option<&'static str> {
    match outcome {
        PollOutcome::Success(_) => None,
        PollOutcome::Auth => Some("auth"),
        PollOutcome::NoCredentials => Some("no-credentials"),
        PollOutcome::NoCurl => Some("no-curl"),
        PollOutcome::Transient => Some("network"),
    }
}

/// Sleep until the next cycle. NoCredentials/NoCurl are pure local checks —
/// no request was made, so there is nothing to back off from, and staying at
/// the normal interval means a user who signs in to Claude Code (or whose
/// curl appears) is noticed within a minute instead of up to 30.
fn next_delay(outcome: &PollOutcome, consecutive_failures: u32) -> Duration {
    match outcome {
        PollOutcome::Success(_) | PollOutcome::NoCredentials | PollOutcome::NoCurl => {
            POLL_INTERVAL
        }
        PollOutcome::Auth | PollOutcome::Transient => backoff_delay(consecutive_failures),
    }
}

pub fn spawn() {
    std::thread::spawn(|| {
        let mut consecutive_failures: u32 = 0;
        loop {
            let outcome = poll_once();
            match &outcome {
                PollOutcome::Success(rate_limits) => {
                    consecutive_failures = 0;
                    let state = State {
                        schema_version: SCHEMA_VERSION,
                        captured_at: now_rfc3339(),
                        rate_limits: Some(rate_limits.clone()),
                        model: None,
                        context_window: None,
                        session_id: None,
                        transcript_path: None,
                        cli_version: None,
                    };
                    let _ = write_state_atomic(&clawdometer_core::paths::live_path(), &state);
                    let _ = std::fs::remove_file(clawdometer_core::paths::poll_error_path());
                }
                outcome => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    if let Some(kind) = error_kind(outcome) {
                        write_poll_error(kind);
                    }
                }
            }
            std::thread::sleep(next_delay(&outcome, consecutive_failures));
        }
    });
}

fn poll_once() -> PollOutcome {
    // curl.exe is the only transport; without it every request would fail in
    // a way indistinguishable from a network outage. Only meaningful as an
    // absolute path — a bare "curl.exe" (no SystemRoot) resolves via PATH at
    // spawn time.
    let curl = clawdometer_core::paths::system32_exe("curl.exe");
    if curl.is_absolute() && !curl.exists() {
        return PollOutcome::NoCurl;
    }
    let cred_path = clawdometer_core::paths::claude_credentials_path();
    let raw = match read_credentials(&cred_path) {
        Ok(raw) => raw,
        Err(outcome) => return outcome,
    };
    // Unparseable content still maps to NoCredentials: whether the file is
    // corrupt or holds no OAuth grant, signing in to Claude Code rewrites it.
    let Ok(mut creds) =
        serde_json::from_str::<serde_json::Value>(raw.trim_start_matches('\u{feff}'))
    else {
        return PollOutcome::NoCredentials;
    };
    let mut refresh_attempted = false;
    if token_expired(&creds, unix_now_ms()) {
        refresh_attempted = true;
        if let Some(fresh) = refresh_and_persist(&cred_path, &creds) {
            creds = fresh;
        }
        // Refresh failure falls through to try the stored token anyway —
        // expiresAt could be wrong, and a failed GET costs nothing extra.
    }
    // File exists but has no OAuth token (e.g. API-key-only setup): signing
    // in to Claude Code is the fix, same as a missing file.
    let Some(token) = access_token(&creds) else {
        return PollOutcome::NoCredentials;
    };
    match fetch(&token) {
        None => PollOutcome::Transient, // curl spawn failure or no response
        Some((200, body)) => match parse_usage(&body) {
            Some(rl) => PollOutcome::Success(rl),
            None => PollOutcome::Transient,
        },
        Some((401 | 403, _)) if !refresh_attempted => {
            // The stored expiresAt lied (token revoked server-side, clock
            // skew, credentials restored from backup): refresh once
            // regardless of the clock, then retry the GET once.
            let Some(fresh) = refresh_and_persist(&cred_path, &creds) else {
                return PollOutcome::Auth;
            };
            let Some(token) = access_token(&fresh) else {
                return PollOutcome::Auth;
            };
            match fetch(&token) {
                Some((200, body)) => match parse_usage(&body) {
                    Some(rl) => PollOutcome::Success(rl),
                    None => PollOutcome::Transient,
                },
                Some((401 | 403, _)) => PollOutcome::Auth,
                _ => PollOutcome::Transient,
            }
        }
        Some((401 | 403, _)) => PollOutcome::Auth,
        Some(_) => PollOutcome::Transient, // 429 / 5xx: back off and retry
    }
}

/// Refresh the grant and write the rotated tokens back, but only if the file
/// still holds the refresh token we consumed (see write_credentials_if_current).
/// Returns the fresh credentials for in-memory use either way — a valid access
/// token is valid regardless of who won the disk write.
fn refresh_and_persist(
    cred_path: &std::path::Path,
    creds: &serde_json::Value,
) -> Option<serde_json::Value> {
    let consumed = creds.pointer("/claudeAiOauth/refreshToken")?.as_str()?.to_string();
    let fresh = refresh(creds, unix_now_ms())?;
    persist_with_retry(cred_path, &fresh, &consumed, Duration::from_millis(200));
    Some(fresh)
}

/// The refresh POST already rotated the token server-side, so a rotated
/// refresh token that never reaches disk is stranded in this cycle's memory —
/// the disk (and Claude Code) keep the consumed one, and the next refresh from
/// either party can fail, signing the user out. Retry brief I/O failures (AV
/// lock, writer mid-write); stop as soon as the disk provably moved on.
fn persist_with_retry(
    path: &std::path::Path,
    fresh: &serde_json::Value,
    consumed_refresh_token: &str,
    retry_delay: Duration,
) -> bool {
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(retry_delay);
        }
        match write_credentials_if_current(path, fresh, consumed_refresh_token) {
            PersistOutcome::Persisted => return true,
            PersistOutcome::Superseded => return false,
            PersistOutcome::Failed => {}
        }
    }
    false
}

/// The raw credentials file. A missing file means no sign-in — installing or
/// opening Claude Code creates it. Any other read error (sharing violation
/// from a writer mid-write, AV lock, ACL) says nothing about sign-in state:
/// the file is there, this cycle just couldn't read it, so it must not tell
/// the user to sign in.
fn read_credentials(path: &std::path::Path) -> Result<String, PollOutcome> {
    std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            PollOutcome::NoCredentials
        } else {
            PollOutcome::Transient
        }
    })
}

/// Failure marker for the HUD ({"kind": "auth" | "network" |
/// "no-credentials" | "no-curl"}), deleted on the next successful poll.
/// Content is timestamp-free so repeated identical failures don't churn the
/// state watcher.
fn write_poll_error(kind: &str) {
    let path = clawdometer_core::paths::poll_error_path();
    let Some(dir) = path.parent() else { return };
    let _ = std::fs::create_dir_all(dir);
    let body = serde_json::json!({ "kind": kind }).to_string();
    let _ = tempfile::NamedTempFile::new_in(dir).and_then(|mut tmp| {
        tmp.write_all(body.as_bytes())?;
        tmp.persist(&path).map_err(|e| e.error)?;
        Ok(())
    });
}

fn unix_now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// Split curl's `--write-out "\n%{http_code}"` trailer off the response body.
/// None when there is no parsable status or curl reported 000 (no response).
fn split_status(stdout: &str) -> Option<(u16, String)> {
    let (body, code) = stdout.rsplit_once('\n')?;
    let code: u16 = code.trim().parse().ok()?;
    if code == 0 {
        return None;
    }
    Some((code, body.to_string()))
}

/// Sleep before the next poll: the normal interval after a success or first
/// failure, doubling per consecutive failure, capped at 30 minutes. Keeps a
/// dead refresh token from hammering the auth endpoint every minute forever.
fn backoff_delay(consecutive_failures: u32) -> Duration {
    let doublings = consecutive_failures.saturating_sub(1).min(6);
    POLL_INTERVAL.saturating_mul(1 << doublings).min(MAX_BACKOFF)
}

/// Result of one attempt to persist rotated credentials.
enum PersistOutcome {
    Persisted,
    /// Disk holds a different refresh token (or the file is gone): Claude
    /// Code rotated first and its copy is the live one — ours must not
    /// clobber it, and there is nothing to retry.
    Superseded,
    /// Read, parse, or write trouble (AV lock, writer mid-write, disk): the
    /// rotated token reached nobody's disk yet, so retrying is worthwhile.
    Failed,
}

/// Persist refreshed credentials only if the file still holds the refresh
/// token we consumed — a check against Claude Code rotating the tokens itself
/// between our read and this write. Only a *provably different* token on disk
/// means ours is stale; an unreadable or torn file is an I/O problem to retry
/// (see persist_with_retry), not evidence the disk is newer.
fn write_credentials_if_current(
    path: &std::path::Path,
    fresh: &serde_json::Value,
    consumed_refresh_token: &str,
) -> PersistOutcome {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return PersistOutcome::Superseded,
        Err(_) => return PersistOutcome::Failed,
    };
    let Ok(on_disk) =
        serde_json::from_str::<serde_json::Value>(raw.trim_start_matches('\u{feff}'))
    else {
        return PersistOutcome::Failed;
    };
    let unchanged = on_disk.pointer("/claudeAiOauth/refreshToken").and_then(|v| v.as_str())
        == Some(consumed_refresh_token);
    if !unchanged {
        return PersistOutcome::Superseded;
    }
    if write_credentials_atomic(path, fresh).is_ok() {
        PersistOutcome::Persisted
    } else {
        PersistOutcome::Failed
    }
}

/// Expired (or about to) per the stored `expiresAt` (epoch millis). Missing
/// or malformed expiry means "not expired": try the token as-is.
fn token_expired(creds: &serde_json::Value, now_ms: i64) -> bool {
    creds
        .pointer("/claudeAiOauth/expiresAt")
        .and_then(|v| v.as_i64())
        .is_some_and(|at| at <= now_ms + EXPIRY_MARGIN_MS)
}

/// The stored access token, rejected unless it is non-empty and confined to
/// the JWT/base64url alphabet — it is interpolated into a quoted curl config
/// directive, so a `"` or newline would escape into arbitrary curl config.
fn access_token(creds: &serde_json::Value) -> Option<String> {
    let token = creds.pointer("/claudeAiOauth/accessToken")?.as_str()?;
    let valid = !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'+' | b'/' | b'=' | b'-'));
    if valid {
        Some(token.to_string())
    } else {
        None
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
        "--proto",
        "=https",
        "--tlsv1.2",
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
/// the machine). Returns (http status, body): no `--fail`, so 401/429/5xx
/// bodies come back with their real status instead of collapsing into one
/// indistinguishable failure. The explicit UA matters here too — Cloudflare
/// answers 429 instead of 401 to curl's default UA on bad-auth requests.
fn fetch(token: &str) -> Option<(u16, String)> {
    let mut cmd = Command::new(clawdometer_core::paths::system32_exe("curl.exe"));
    cmd.args([
        "--silent",
        "--proto",
        "=https",
        "--tlsv1.2",
        "--max-time",
        "15",
        "-A",
        USER_AGENT,
        "--write-out",
        "\n%{http_code}",
        "--config",
        "-",
        USAGE_URL,
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
    split_status(&String::from_utf8(out.stdout).ok()?)
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
    fn splits_body_and_status_code() {
        assert_eq!(split_status("{\"a\":1}\n200"), Some((200, "{\"a\":1}".to_string())));
        assert_eq!(split_status("\n401"), Some((401, String::new())));
    }

    #[test]
    fn split_status_rejects_missing_or_unreachable() {
        assert_eq!(split_status("no trailing status"), None);
        assert_eq!(split_status("body\n000"), None); // curl: no response received
        assert_eq!(split_status(""), None);
    }

    #[test]
    fn error_kind_names_each_failure_and_success_has_none() {
        assert_eq!(error_kind(&PollOutcome::NoCredentials), Some("no-credentials"));
        assert_eq!(error_kind(&PollOutcome::NoCurl), Some("no-curl"));
        assert_eq!(error_kind(&PollOutcome::Auth), Some("auth"));
        assert_eq!(error_kind(&PollOutcome::Transient), Some("network"));
        let ok = PollOutcome::Success(RateLimits { five_hour: None, seven_day: None });
        assert_eq!(error_kind(&ok), None);
    }

    #[test]
    fn local_failures_poll_at_normal_interval_network_failures_back_off() {
        // NoCredentials/NoCurl never touch the network, so backing off would
        // only delay noticing the user's fix (e.g. signing in to Claude Code).
        assert_eq!(next_delay(&PollOutcome::NoCredentials, 10), POLL_INTERVAL);
        assert_eq!(next_delay(&PollOutcome::NoCurl, 10), POLL_INTERVAL);
        assert_eq!(next_delay(&PollOutcome::Transient, 3), Duration::from_secs(240));
        assert_eq!(next_delay(&PollOutcome::Auth, 10), MAX_BACKOFF);
        let ok = PollOutcome::Success(RateLimits { five_hour: None, seven_day: None });
        assert_eq!(next_delay(&ok, 0), POLL_INTERVAL);
    }

    #[test]
    fn backoff_starts_at_poll_interval_doubles_and_caps() {
        assert_eq!(backoff_delay(0), Duration::from_secs(60));
        assert_eq!(backoff_delay(1), Duration::from_secs(60));
        assert_eq!(backoff_delay(2), Duration::from_secs(120));
        assert_eq!(backoff_delay(3), Duration::from_secs(240));
        assert_eq!(backoff_delay(10), Duration::from_secs(1800));
        assert_eq!(backoff_delay(u32::MAX), Duration::from_secs(1800));
    }

    #[test]
    fn rejects_tokens_with_config_breaking_characters() {
        // A token is interpolated into a curl config line; anything outside
        // the base64url/JWT alphabet could escape the quoted directive.
        for bad in ["tok\"123", "tok\n123", "tok 123", "tok\\123"] {
            let c = creds(&format!(
                r#"{{"claudeAiOauth": {{"accessToken": {}}}}}"#,
                serde_json::json!(bad)
            ));
            assert!(access_token(&c).is_none(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn persists_only_when_disk_refresh_token_is_the_one_consumed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        std::fs::write(
            &path,
            r#"{"claudeAiOauth": {"refreshToken": "old-r", "accessToken": "old-a"}}"#,
        )
        .unwrap();
        let fresh = creds(r#"{"claudeAiOauth": {"refreshToken": "new-r", "accessToken": "new-a"}}"#);
        // Disk changed under us (Claude Code rotated first): must not clobber.
        assert!(matches!(
            write_credentials_if_current(&path, &fresh, "someone-elses-r"),
            PersistOutcome::Superseded
        ));
        assert!(std::fs::read_to_string(&path).unwrap().contains("old-a"));
        // Disk still holds the token we consumed: persist.
        assert!(matches!(
            write_credentials_if_current(&path, &fresh, "old-r"),
            PersistOutcome::Persisted
        ));
        assert!(std::fs::read_to_string(&path).unwrap().contains("new-a"));
    }

    #[test]
    fn does_not_create_credentials_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        let fresh = creds(r#"{"claudeAiOauth": {"refreshToken": "new-r"}}"#);
        assert!(matches!(
            write_credentials_if_current(&path, &fresh, "old-r"),
            PersistOutcome::Superseded
        ));
        assert!(!path.exists());
    }

    #[test]
    fn missing_credentials_file_reads_as_no_credentials() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_credentials(&dir.path().join("nope.json")),
            Err(PollOutcome::NoCredentials)
        ));
    }

    #[test]
    fn readable_credentials_pass_through() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        std::fs::write(&path, "{}").unwrap();
        assert!(matches!(read_credentials(&path).as_deref(), Ok("{}")));
    }

    #[cfg(windows)]
    #[test]
    fn locked_credentials_file_reads_as_transient_not_auth() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        std::fs::write(&path, "{}").unwrap();
        // share_mode(0): exclusive open — every other open gets a sharing
        // violation, which is what an AV scan or a writer mid-write looks
        // like. The user IS signed in, so this must not be auth-shaped.
        let _lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        assert!(matches!(read_credentials(&path), Err(PollOutcome::Transient)));
    }

    #[test]
    fn persist_reports_torn_disk_file_as_retryable() {
        // A torn read is a non-atomic writer mid-write, not proof the disk is
        // newer — must come back Failed (retry) rather than Superseded (drop
        // the rotated token on the floor).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        std::fs::write(&path, "{ torn").unwrap();
        let fresh = creds(r#"{"claudeAiOauth": {"refreshToken": "new-r"}}"#);
        assert!(matches!(
            write_credentials_if_current(&path, &fresh, "old-r"),
            PersistOutcome::Failed
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ torn");
    }

    #[cfg(windows)]
    #[test]
    fn persist_reports_locked_file_as_retryable() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        std::fs::write(&path, r#"{"claudeAiOauth": {"refreshToken": "old-r"}}"#).unwrap();
        let _lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        let fresh = creds(r#"{"claudeAiOauth": {"refreshToken": "new-r"}}"#);
        assert!(matches!(
            write_credentials_if_current(&path, &fresh, "old-r"),
            PersistOutcome::Failed
        ));
    }

    #[test]
    fn persist_with_retry_recovers_when_transient_failure_clears() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        std::fs::write(&path, "{ torn").unwrap();
        let fresh = creds(r#"{"claudeAiOauth": {"refreshToken": "new-r", "accessToken": "new-a"}}"#);
        let fixer_path = path.clone();
        let fixer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            std::fs::write(&fixer_path, r#"{"claudeAiOauth": {"refreshToken": "old-r"}}"#).unwrap();
        });
        assert!(persist_with_retry(&path, &fresh, "old-r", Duration::from_millis(25)));
        fixer.join().unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("new-a"));
    }

    #[test]
    fn persist_with_retry_stops_immediately_when_superseded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        std::fs::write(&path, r#"{"claudeAiOauth": {"refreshToken": "someone-elses-r"}}"#).unwrap();
        let fresh = creds(r#"{"claudeAiOauth": {"refreshToken": "new-r"}}"#);
        assert!(!persist_with_retry(&path, &fresh, "old-r", Duration::ZERO));
        assert!(std::fs::read_to_string(&path).unwrap().contains("someone-elses-r"));
    }

    #[test]
    fn apply_refresh_rejects_bad_responses() {
        let c = creds(r#"{"claudeAiOauth": {"accessToken": "old-a"}}"#);
        assert!(apply_refresh(&c, "not json", 0).is_none());
        assert!(apply_refresh(&c, "{}", 0).is_none());
        assert!(apply_refresh(&c, r#"{"access_token": ""}"#, 0).is_none());
    }
}
