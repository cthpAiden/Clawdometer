use std::path::{Path, PathBuf};
use std::time::Duration;

use clawdometer_core::state::{merge, zero_expired_windows};
use tauri::{AppHandle, Emitter};

pub const STATE_EVENT: &str = "state-updated";

// A transcript is only inspected if it was touched within this many secs — a
// bound so a session abandoned mid-turn can't pin `working` on forever. Set
// well above the longest realistic tool-free "thinking" stretch, since that
// writes nothing to the transcript until it completes.
const ACTIVE_CAP_SECS: i64 = 300;
// Bytes read from a transcript's tail to find its last message entry. Ample for
// any single JSONL line; a partial leading fragment just fails to parse.
const TAIL_BYTES: u64 = 512 * 1024;

pub fn build_payload(
    state_path: &Path,
    live_path: &Path,
    transcripts_dir: &Path,
    now_epoch_secs: i64,
) -> serde_json::Value {
    // `working` = some Claude Code session is mid-turn, judged by the turn state
    // in its transcript (not by file age — a transcript is silent during a long
    // tool-free "thinking" stretch). Fires on every client, GUIs included, and
    // never keys off live.json's 60s /usage poll.
    let working = any_session_generating(transcripts_dir, now_epoch_secs);
    // state.json (statusline hook) merged with live.json (headless /usage
    // refresh) — whichever snapshot is newer wins.
    let state = merge(
        clawdometer_core::state::read_state(state_path),
        clawdometer_core::state::read_state(live_path),
    )
    .map(|mut s| {
        // A window whose reset time has passed is 0% until the next request
        // opens a new one — without this, an idle machine would show the last
        // snapshot's percentage forever.
        if let Some(rl) = s.rate_limits.as_mut() {
            zero_expired_windows(rl, now_epoch_secs);
        }
        s
    });
    // The webview gets only what it renders: rate_limits + captured_at.
    // session_id / transcript_path / model / cli_version stay on this side
    // of the IPC boundary.
    let state = state
        .map(|s| serde_json::json!({ "captured_at": s.captured_at, "rate_limits": s.rate_limits }))
        .unwrap_or(serde_json::Value::Null);
    serde_json::json!({ "state": state, "working": working })
}

/// True while some Claude Code session is mid-turn. Scans transcripts under
/// `dir` (`<project>/<session>.jsonl`); a file is inspected only if touched
/// within ACTIVE_CAP_SECS (cheap stat first), then judged structurally by its
/// last message. Reads file contents locally to derive one bool — never over
/// the network, never exposed past this process.
fn any_session_generating(dir: &Path, now_epoch_secs: i64) -> bool {
    let Ok(projects) = std::fs::read_dir(dir) else { return false };
    for proj in projects.flatten() {
        let Ok(files) = std::fs::read_dir(proj.path()) else { continue };
        for f in files.flatten() {
            let path = f.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(m) = mtime_secs(&f) else { continue };
            // stale beyond the cap, or a future mtime (clock skew) => skip
            if !(0..=ACTIVE_CAP_SECS).contains(&(now_epoch_secs - m)) {
                continue;
            }
            if tail_indicates_generating(&read_tail(&path, TAIL_BYTES)) {
                return true;
            }
        }
    }
    false
}

/// Scan a transcript tail newest-line-first for the last real message entry
/// (client metadata lines like `last-prompt`/`custom-title`, and any truncated
/// leading fragment, are skipped). A completed assistant turn (terminal
/// stop_reason) is idle; an unanswered user/tool_result or an assistant still
/// paused on a tool is generating.
fn tail_indicates_generating(tail: &str) -> bool {
    for line in tail.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => {
                let terminal = v
                    .pointer("/message/stop_reason")
                    .and_then(|s| s.as_str())
                    .is_some_and(is_terminal_stop_reason);
                return !terminal;
            }
            // A user entry is an in-flight turn — unless it's scaffolding Claude
            // Code writes with no reply pending (the /compact summary, slash-
            // command echoes). Those must not pin `working` on, so skip past them
            // to the real last message.
            Some("user") if is_synthetic_user_entry(&v) => continue,
            // Esc mid-turn: the interrupt marker terminates the turn — no
            // reply is coming. Return, don't skip: the entry underneath is
            // the interrupted (mid-turn) one and would pin `working` on.
            Some("user") if is_interrupt_entry(&v) => return false,
            Some("user") => return true,
            // Terminal system markers: the turn is over even if no assistant
            // entry with a terminal stop_reason was written (turns can end on
            // a tool_result). stop_hook_summary = stop hook ran after the turn;
            // compact_boundary = compaction finished between turns.
            Some("system")
                if matches!(
                    v.get("subtype").and_then(|s| s.as_str()),
                    Some("stop_hook_summary" | "compact_boundary")
                ) =>
            {
                return false
            }
            _ => continue,
        }
    }
    false
}

/// A `user`-typed transcript entry that isn't a prompt awaiting a reply: the
/// post-/compact summary, or a slash-command echo (`<command-name>`,
/// `<local-command-*>`). These land after a turn completes, so treating them as
/// mid-turn would blip the HUD on while idle.
fn is_synthetic_user_entry(v: &serde_json::Value) -> bool {
    if v.get("isCompactSummary").and_then(|b| b.as_bool()) == Some(true) {
        return true;
    }
    v.pointer("/message/content")
        .and_then(|c| c.as_str())
        .is_some_and(|s| {
            s.starts_with("<command-name>") || s.starts_with("<local-command-")
        })
}

/// A `user`-typed entry Claude Code writes when the user interrupts a turn
/// (Esc). Text is "[Request interrupted by user]" or the "... for tool use]"
/// variant, either as a bare string or as a text block in an array (possibly
/// after a tool_result block).
fn is_interrupt_entry(v: &serde_json::Value) -> bool {
    const MARKER: &str = "[Request interrupted by user";
    match v.pointer("/message/content") {
        Some(serde_json::Value::String(s)) => s.starts_with(MARKER),
        Some(serde_json::Value::Array(blocks)) => blocks.iter().any(|b| {
            b.get("type").and_then(|t| t.as_str()) == Some("text")
                && b.get("text").and_then(|t| t.as_str()).is_some_and(|s| s.starts_with(MARKER))
        }),
        _ => false,
    }
}

fn is_terminal_stop_reason(stop_reason: &str) -> bool {
    matches!(stop_reason, "end_turn" | "stop_sequence" | "max_tokens")
}

/// Last `max_bytes` of a file as lossy UTF-8 (empty on any IO error). Seeking to
/// the end avoids reading multi-MB transcripts in full every poll.
fn read_tail(path: &Path, max_bytes: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if f.seek(SeekFrom::Start(len.saturating_sub(max_bytes))).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = f.take(max_bytes).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn mtime_secs(entry: &std::fs::DirEntry) -> Option<i64> {
    let modified = entry.metadata().ok()?.modified().ok()?;
    let secs = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(secs as i64)
}

fn now_epoch_secs() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// notify watcher on ~/.clawdometer + 2s fallback poll. Emits only when the
/// serialized payload's state differs from the last emission (poll path) or
/// the FS reports a change (watch path) — zeroing makes the payload cross a
/// window's reset time exactly once, so that transition re-emits too. Never
/// panics the app: watch setup failure degrades to poll-only.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let state_path: PathBuf = clawdometer_core::paths::state_path();
        let live_path: PathBuf = clawdometer_core::paths::live_path();
        let transcripts_dir: PathBuf = clawdometer_core::paths::projects_dir();
        let dir = state_path.parent().map(Path::to_path_buf);

        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let _watcher = dir.and_then(|dir| {
            use notify::Watcher;
            let tx = tx.clone();
            let mut w = notify::recommended_watcher(move |_res| {
                let _ = tx.send(());
            })
            .ok()?;
            std::fs::create_dir_all(&dir).ok();
            w.watch(&dir, notify::RecursiveMode::NonRecursive).ok()?;
            Some(w)
        });

        // initial emission so the UI renders immediately
        let payload = build_payload(&state_path, &live_path, &transcripts_dir, now_epoch_secs());
        let mut last_payload = payload.clone();
        let _ = app.emit(STATE_EVENT, &payload);
        update_tooltip(&app, &payload);

        // The webview's event listener attaches asynchronously, so early
        // emissions can be lost. Re-emit unconditionally for the first few
        // ticks so a restarted HUD renders pre-existing state.
        let mut startup_grace: u32 = 5;

        loop {
            // wake on FS event or every 2s (debounce fallback poll)
            let _ = rx.recv_timeout(Duration::from_secs(2));
            let payload = build_payload(&state_path, &live_path, &transcripts_dir, now_epoch_secs());
            // The payload is timestamp-free, so identical data never re-emits.
            if startup_grace > 0 || payload != last_payload {
                startup_grace = startup_grace.saturating_sub(1);
                last_payload = payload.clone();
                let _ = app.emit(STATE_EVENT, &payload);
                update_tooltip(&app, &payload);
            }
        }
    });
}

/// Tray tooltip: percentages when there's data; otherwise how to get some —
/// the statusline hook only delivers data while Claude Code runs, so a hidden
/// HUD isn't the only place a first run is explained.
fn tooltip_text(payload: &serde_json::Value) -> String {
    match (
        payload.pointer("/state/rate_limits/five_hour/used_percentage").and_then(|v| v.as_i64()),
        payload.pointer("/state/rate_limits/seven_day/used_percentage").and_then(|v| v.as_i64()),
    ) {
        (Some(fh), Some(sd)) => format!("5h {fh}% · 7d {sd}%"),
        (Some(fh), None) => format!("5h {fh}%"),
        (None, Some(sd)) => format!("7d {sd}%"),
        (None, None) => "Clawdometer — waiting for data, open Claude Code".into(),
    }
}

fn update_tooltip(app: &AppHandle, payload: &serde_json::Value) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(tooltip_text(payload)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawdometer_core::schema::{LimitWindow, RateLimits};
    use clawdometer_core::state::{State, SCHEMA_VERSION};

    fn snapshot(captured_at: &str, pct: Option<i64>, resets_at: i64) -> State {
        State {
            schema_version: SCHEMA_VERSION,
            captured_at: captured_at.into(),
            rate_limits: pct.map(|p| RateLimits {
                five_hour: Some(LimitWindow { used_percentage: p, resets_at }),
                seven_day: None,
            }),
            model: None,
            context_window: None,
            session_id: None,
            transcript_path: None,
            cli_version: None,
        }
    }

    #[test]
    fn payload_is_null_state_when_file_missing_or_torn() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("state.json");
        let live = dir.path().join("live.json");
        let noproj = dir.path().join("noproj");
        assert!(build_payload(&missing, &live, &noproj, 0)["state"].is_null());
        std::fs::write(&missing, "{ torn").unwrap();
        assert!(build_payload(&missing, &live, &noproj, 0)["state"].is_null());
    }

    #[test]
    fn payload_carries_state_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let raw = include_str!("../../../crates/core/tests/fixtures/stdin-with-limits.json");
        let input = clawdometer_core::schema::parse_statusline_input(raw).unwrap();
        let state = clawdometer_core::state::State::from_input(&input, "t".into());
        clawdometer_core::state::write_state_atomic(&path, &state).unwrap();
        let payload = build_payload(&path, &dir.path().join("live.json"), &dir.path().join("noproj"), 0);
        assert_eq!(payload["state"]["rate_limits"]["five_hour"]["used_percentage"], 1);
        assert_eq!(payload["state"]["captured_at"], "t");
    }

    #[test]
    fn newer_live_refresh_snapshot_wins() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("state.json");
        let live_path = dir.path().join("live.json");
        let s = snapshot("2026-07-12T00:00:00Z", Some(97), 9_999_999_999);
        let l = snapshot("2026-07-12T00:05:00Z", Some(99), 9_999_999_999);
        clawdometer_core::state::write_state_atomic(&state_path, &s).unwrap();
        clawdometer_core::state::write_state_atomic(&live_path, &l).unwrap();
        let payload = build_payload(&state_path, &live_path, &dir.path().join("noproj"), 0);
        assert_eq!(payload["state"]["rate_limits"]["five_hour"]["used_percentage"], 99);
        assert_eq!(payload["state"]["captured_at"], "2026-07-12T00:05:00Z");
    }

    #[test]
    fn payload_exposes_only_what_the_ui_renders() {
        // The webview should never receive session_id / transcript_path /
        // model / cli_version — it renders rate_limits + captured_at only.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = snapshot("t", Some(42), 9_999_999_999);
        state.session_id = Some("sess-secret".into());
        state.transcript_path = Some("C:\\transcripts\\x.jsonl".into());
        state.cli_version = Some("2.1.205".into());
        clawdometer_core::state::write_state_atomic(&path, &state).unwrap();
        let payload = build_payload(&path, &dir.path().join("live.json"), &dir.path().join("noproj"), 0);
        let obj = payload["state"].as_object().unwrap();
        assert_eq!(obj.keys().collect::<Vec<_>>(), ["captured_at", "rate_limits"]);
        assert!(payload.get("poll_error").is_none(), "dead field must be gone");
        // top level carries only the render state + the activity flag
        let top = payload.as_object().unwrap();
        assert_eq!(top.keys().collect::<Vec<_>>(), ["state", "working"]);
    }

    #[test]
    fn payload_zeroes_window_once_its_reset_time_passes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let live = dir.path().join("live.json");
        let noproj = dir.path().join("noproj");
        let state = snapshot("t", Some(42), 1_000);
        clawdometer_core::state::write_state_atomic(&path, &state).unwrap();
        // before the reset: the snapshot's percentage
        let before = build_payload(&path, &live, &noproj, 999);
        assert_eq!(before["state"]["rate_limits"]["five_hour"]["used_percentage"], 42);
        // at/after the reset: derived 0%, resets_at kept for the UI label
        let after = build_payload(&path, &live, &noproj, 1_000);
        assert_eq!(after["state"]["rate_limits"]["five_hour"]["used_percentage"], 0);
        assert_eq!(after["state"]["rate_limits"]["five_hour"]["resets_at"], 1_000);
        // the transition changes the payload exactly once, so the watcher
        // loop's compare re-emits at the boundary
        assert_ne!(before, after);
        assert_eq!(after, build_payload(&path, &live, &noproj, 2_000));
    }

    #[test]
    fn tooltip_explains_how_to_get_data() {
        let t = tooltip_text(&serde_json::json!({ "state": null }));
        assert_eq!(t, "Clawdometer — waiting for data, open Claude Code");
    }

    #[test]
    fn tooltip_shows_percentages_when_present() {
        let payload = serde_json::json!({
            "state": { "rate_limits": {
                "five_hour": { "used_percentage": 4 },
                "seven_day": { "used_percentage": 16 },
            }},
        });
        assert_eq!(tooltip_text(&payload), "5h 4% · 7d 16%");
    }

    // Representative transcript entries. stop_reason + type are standard
    // Anthropic/Claude Code fields; the metadata types are client chrome we skip.
    const ASSISTANT_DONE: &str =
        r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text"}]}}"#;
    const ASSISTANT_TOOL: &str =
        r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use"}]}}"#;
    const USER_MSG: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"text"}]}}"#;
    const META_TITLE: &str = r#"{"type":"custom-title"}"#;
    const META_PROMPT: &str = r#"{"type":"last-prompt"}"#;
    // Synthetic `user` entries Claude Code writes after /compact: the summary
    // (isCompactSummary) and the slash-command echoes. None is an in-flight
    // prompt — the previous turn already completed.
    const COMPACT_SUMMARY: &str =
        r#"{"type":"user","isCompactSummary":true,"message":{"role":"user","content":"This session is being continued..."}}"#;
    const CMD_CAVEAT: &str =
        r#"{"type":"user","message":{"role":"user","content":"<local-command-caveat>Caveat: ...</local-command-caveat>"}}"#;
    const CMD_NAME: &str =
        r#"{"type":"user","message":{"role":"user","content":"<command-name>/compact</command-name>"}}"#;
    const CMD_STDOUT: &str =
        r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>Compacted </local-command-stdout>"}}"#;
    // A tool result is `user`-typed with array content — mid-turn it means the
    // assistant is about to continue, but a turn can also END on one (no
    // terminal assistant entry ever written; observed in real transcripts).
    const TOOL_RESULT: &str =
        r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_x","type":"tool_result","content":"ok"}]}}"#;
    // Terminal system markers: turn is over / compaction finished.
    const STOP_HOOK: &str = r#"{"type":"system","subtype":"stop_hook_summary","level":"suggestion"}"#;
    const BOUNDARY: &str =
        r#"{"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}"#;
    // Esc mid-turn: Claude Code records the interrupt as a `user` entry. No
    // reply is coming, so it terminates the turn. Real shapes: a lone text
    // block, or a tool_result followed by the tool-use variant of the marker.
    const INTERRUPT: &str =
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
    const INTERRUPT_TOOL: &str =
        r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_x","type":"tool_result","content":"ok"},{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#;

    #[test]
    fn terminal_stop_reasons() {
        assert!(is_terminal_stop_reason("end_turn"));
        assert!(is_terminal_stop_reason("stop_sequence"));
        assert!(is_terminal_stop_reason("max_tokens"));
        assert!(!is_terminal_stop_reason("tool_use"));
        assert!(!is_terminal_stop_reason(""));
    }

    #[test]
    fn generating_iff_last_real_message_is_unfinished() {
        // completed turn, even with trailing client metadata after it => idle
        let done = format!("{USER_MSG}\n{ASSISTANT_DONE}\n{META_PROMPT}\n{META_TITLE}\n");
        assert!(!tail_indicates_generating(&done));
        // assistant paused on a tool call => generating
        assert!(tail_indicates_generating(&format!("{ASSISTANT_TOOL}\n{META_TITLE}\n")));
        // user prompt not yet answered (long think, nothing written yet) => generating
        assert!(tail_indicates_generating(&format!("{ASSISTANT_DONE}\n{USER_MSG}\n")));
        // only metadata / empty => idle
        assert!(!tail_indicates_generating(&format!("{META_TITLE}\n")));
        assert!(!tail_indicates_generating(""));
        // a truncated (unparseable) leading fragment is skipped, not fatal
        assert!(tail_indicates_generating(&format!("{{\"type\":\"assi…broken\n{ASSISTANT_TOOL}\n")));
    }

    #[test]
    fn idle_after_compact_despite_synthetic_user_entries() {
        // After /compact the tail is the prior turn's end_turn, then the compact
        // summary and the /compact command echoes — all `user`-typed, none an
        // in-flight prompt. Must read idle, not blip on until the cap.
        let tail = format!(
            "{ASSISTANT_DONE}\n{COMPACT_SUMMARY}\n{CMD_CAVEAT}\n{CMD_NAME}\n{CMD_STDOUT}\n"
        );
        assert!(!tail_indicates_generating(&tail));
    }

    #[test]
    fn genuine_prompt_after_compact_scaffolding_generates() {
        // The user then sends a real prompt after the scaffolding: that IS an
        // in-flight turn and must blip.
        let real = r#"{"type":"user","message":{"role":"user","content":"why is it blipping"}}"#;
        let tail = format!("{ASSISTANT_DONE}\n{COMPACT_SUMMARY}\n{CMD_STDOUT}\n{real}\n");
        assert!(tail_indicates_generating(&tail));
    }

    #[test]
    fn idle_after_compact_when_prior_turn_ended_on_tool_result() {
        // Real shape from a live transcript: last turn ended on a tool_result
        // (no end_turn assistant entry), then stop hook ran, then /compact.
        // Walkback past the synthetics must stop at the system markers, not
        // fall through to the tool_result and pin `working` on.
        let tail = format!(
            "{TOOL_RESULT}\n{STOP_HOOK}\n{BOUNDARY}\n{COMPACT_SUMMARY}\n{CMD_CAVEAT}\n{CMD_NAME}\n{CMD_STDOUT}\n"
        );
        assert!(!tail_indicates_generating(&tail));
        // Stop hook alone (no compact) also marks the turn done.
        assert!(!tail_indicates_generating(&format!("{TOOL_RESULT}\n{STOP_HOOK}\n")));
        // But a bare trailing tool_result IS mid-turn — must still blip.
        assert!(tail_indicates_generating(&format!("{ASSISTANT_TOOL}\n{TOOL_RESULT}\n")));
    }

    #[test]
    fn idle_after_user_interrupts_mid_turn() {
        // The user pressed Esc mid-turn: the interrupt entry is the last real
        // message (trailing client metadata after it). Must read idle, not pin
        // `working` on until the cap — the walkback must also NOT skip past it
        // to the mid-turn assistant entry underneath.
        let tail = format!("{ASSISTANT_TOOL}\n{INTERRUPT}\n{META_PROMPT}\n{META_TITLE}\n");
        assert!(!tail_indicates_generating(&tail));
        // Tool-use variant: tool_result + interrupt text in one entry.
        let tail = format!("{ASSISTANT_TOOL}\n{INTERRUPT_TOOL}\n");
        assert!(!tail_indicates_generating(&tail));
    }

    /// Write a one-project transcript with the given JSONL body; returns
    /// (projects_root, transcript_mtime_secs). Tests drive `working` by choosing
    /// the `now` they pass to build_payload rather than backdating the file.
    fn transcript(body: &str) -> (tempfile::TempDir, i64) {
        let root = tempfile::tempdir().unwrap();
        let proj = root.path().join("F--proj");
        std::fs::create_dir_all(&proj).unwrap();
        let f = proj.join("sess.jsonl");
        std::fs::write(&f, body).unwrap();
        let m = std::fs::metadata(&f)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        (root, m)
    }

    fn working_at(root: &Path, now: i64) -> bool {
        let dir = tempfile::tempdir().unwrap();
        build_payload(&dir.path().join("state.json"), &dir.path().join("live.json"), root, now)
            ["working"]
            == true
    }

    #[test]
    fn working_true_while_a_session_is_mid_turn() {
        // prior turn done, new prompt sent, Claude thinking (nothing written yet)
        let (root, m) = transcript(&format!("{ASSISTANT_DONE}\n{USER_MSG}\n"));
        assert!(working_at(root.path(), m));
    }

    #[test]
    fn working_false_the_instant_the_turn_completes() {
        // No time-window: a completed assistant turn reads idle immediately,
        // even though the file was just written — this is what kills the tail.
        let (root, m) = transcript(&format!("{USER_MSG}\n{ASSISTANT_DONE}\n{META_TITLE}\n"));
        assert!(!working_at(root.path(), m));
    }

    #[test]
    fn working_false_when_a_mid_turn_session_is_stale_beyond_the_cap() {
        // abandoned/crashed mid-turn: not touched within ACTIVE_CAP_SECS => idle
        let (root, m) = transcript(&format!("{USER_MSG}\n"));
        assert!(working_at(root.path(), m + ACTIVE_CAP_SECS)); // within cap: counts
        assert!(!working_at(root.path(), m + ACTIVE_CAP_SECS + 1)); // past cap: idle
    }

    #[test]
    fn working_false_when_only_live_json_is_fresh() {
        // live.json is the 60s /usage poll; with no transcript there's no turn to
        // be mid, so `working` stays false though the merged snapshot renders it.
        let dir = tempfile::tempdir().unwrap();
        let live_path = dir.path().join("live.json");
        let l = snapshot("2026-07-12T00:00:00Z", Some(88), 9_999_999_999);
        clawdometer_core::state::write_state_atomic(&live_path, &l).unwrap();
        let payload = build_payload(
            &dir.path().join("state.json"),
            &live_path,
            &dir.path().join("noproj"),
            4_000_000_000,
        );
        assert_eq!(payload["working"], false);
        assert_eq!(payload["state"]["rate_limits"]["five_hour"]["used_percentage"], 88);
    }
}
