use clawdometer_core::schema::{parse_statusline_input, LimitWindow, RateLimits};
use clawdometer_core::state::{merge, read_state, write_state_atomic, State};

const PRE: &str = include_str!("fixtures/stdin-pre-response.json");
const FULL: &str = include_str!("fixtures/stdin-with-limits.json");

fn win(pct: i64) -> Option<LimitWindow> {
    Some(LimitWindow { used_percentage: pct, resets_at: 4_000_000_000 })
}

fn snapshot(captured_at: &str, rl: Option<RateLimits>) -> Option<State> {
    Some(State {
        schema_version: 1,
        captured_at: captured_at.into(),
        rate_limits: rl,
        model: None,
        context_window: None,
        session_id: None,
        transcript_path: None,
        cli_version: None,
    })
}

/// The statusline hook never carries fable_week, and it writes a newer
/// snapshot on every prompt. Taking rate_limits as one unit would blank the
/// Fable bar until the next /usage refresh brought it back — a visible flicker
/// on each turn.
#[test]
fn merge_keeps_fable_week_when_a_newer_statusline_snapshot_lacks_it() {
    let live = snapshot(
        "2026-07-13T06:00:00Z",
        Some(RateLimits { five_hour: win(30), seven_day: win(39), fable_week: win(54) }),
    );
    let state = snapshot(
        "2026-07-13T06:05:00Z", // newer: the hook just fired
        Some(RateLimits { five_hour: win(31), seven_day: win(39), fable_week: None }),
    );
    let rl = merge(state, live).unwrap().rate_limits.unwrap();
    assert_eq!(rl.five_hour.unwrap().used_percentage, 31, "newer statusline still wins");
    assert_eq!(rl.fable_week.unwrap().used_percentage, 54, "fable must survive the hook");
}

/// The reverse direction: a fresh refresh must be able to update the value,
/// not just be a fallback for it.
#[test]
fn merge_prefers_fable_week_from_the_newer_refresh() {
    let state = snapshot(
        "2026-07-13T06:00:00Z",
        Some(RateLimits { five_hour: win(30), seven_day: win(39), fable_week: win(54) }),
    );
    let live = snapshot(
        "2026-07-13T06:05:00Z",
        Some(RateLimits { five_hour: win(33), seven_day: win(41), fable_week: win(61) }),
    );
    let rl = merge(state, live).unwrap().rate_limits.unwrap();
    assert_eq!(rl.fable_week.unwrap().used_percentage, 61);
}

#[test]
fn state_from_full_input_matches_spec_shape() {
    let input = parse_statusline_input(FULL).unwrap();
    let state = State::from_input(&input, "2026-07-12T02:02:16Z".into());
    assert_eq!(state.schema_version, 1);
    assert_eq!(state.captured_at, "2026-07-12T02:02:16Z");
    assert_eq!(state.rate_limits.as_ref().unwrap().five_hour.as_ref().unwrap().used_percentage, 1);
    assert_eq!(state.context_window.as_ref().unwrap().used_percentage, 4);
    assert_eq!(state.model.as_ref().unwrap().display_name, "Opus 4.8 (1M context)");
    assert_eq!(state.cli_version.as_deref(), Some("2.1.205"));
    assert!(state.transcript_path.is_some());
}

#[test]
fn state_from_pre_response_input_has_null_limits_and_context() {
    let input = parse_statusline_input(PRE).unwrap();
    let state = State::from_input(&input, "2026-07-12T02:02:03Z".into());
    assert!(state.rate_limits.is_none());
    // used_percentage was null in the dump -> normalized to null context_window
    assert!(state.context_window.is_none());
}

#[test]
fn serialized_state_has_null_not_missing_keys() {
    let input = parse_statusline_input(PRE).unwrap();
    let state = State::from_input(&input, "t".into());
    let value: serde_json::Value = serde_json::to_value(&state).unwrap();
    // Spec: rate_limits and context_window are null when absent, not omitted.
    assert!(value.get("rate_limits").unwrap().is_null());
    assert!(value.get("context_window").unwrap().is_null());
}

#[test]
fn write_read_round_trip_and_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");

    let input = parse_statusline_input(PRE).unwrap();
    let first = State::from_input(&input, "t1".into());
    write_state_atomic(&path, &first).unwrap();

    let input = parse_statusline_input(FULL).unwrap();
    let second = State::from_input(&input, "t2".into());
    write_state_atomic(&path, &second).unwrap(); // overwrite must succeed

    let read = read_state(&path).unwrap();
    assert_eq!(read.captured_at, "t2");
    assert!(read.rate_limits.is_some());
    // no stray temp files left behind
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap()
        .filter(|e| e.as_ref().unwrap().file_name() != "state.json").collect();
    assert!(leftovers.is_empty(), "temp files leaked: {leftovers:?}");
}

#[test]
fn read_state_is_tolerant() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_state(&dir.path().join("missing.json")).is_none());
    let torn = dir.path().join("torn.json");
    std::fs::write(&torn, "{\"schema_version\":1,\"capt").unwrap();
    assert!(read_state(&torn).is_none());
}

#[test]
fn write_creates_parent_dir() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("state.json");
    let input = parse_statusline_input(PRE).unwrap();
    write_state_atomic(&path, &State::from_input(&input, "t".into())).unwrap();
    assert!(read_state(&path).is_some());
}

use clawdometer_core::state::{render_statusline, zero_expired_windows};

#[test]
fn zero_expired_windows_zeroes_only_past_resets() {
    let input = parse_statusline_input(FULL).unwrap();
    let mut state = State::from_input(&input, "t".into());
    let rl = state.rate_limits.as_mut().unwrap();
    rl.five_hour.as_mut().unwrap().resets_at = 1_000;
    rl.seven_day.as_mut().unwrap().resets_at = 2_000;

    // Before either reset: untouched.
    zero_expired_windows(rl, 999);
    assert_eq!(rl.five_hour.as_ref().unwrap().used_percentage, 1);
    // 5h reset passed (boundary inclusive), 7d still ahead.
    zero_expired_windows(rl, 1_000);
    assert_eq!(rl.five_hour.as_ref().unwrap().used_percentage, 0);
    assert_eq!(rl.five_hour.as_ref().unwrap().resets_at, 1_000, "resets_at kept for UI label");
    assert_eq!(rl.seven_day.as_ref().unwrap().used_percentage, 5);
    // Both passed.
    zero_expired_windows(rl, 2_001);
    assert_eq!(rl.seven_day.as_ref().unwrap().used_percentage, 0);
}

#[test]
fn renders_line_with_limits() {
    let input = parse_statusline_input(FULL).unwrap();
    let state = State::from_input(&input, "t".into());
    assert_eq!(render_statusline(&state), "[Opus 4.8 (1M context)] 5h 1% · 7d 5%");
}

#[test]
fn renders_pending_line_without_limits() {
    let input = parse_statusline_input(PRE).unwrap();
    let state = State::from_input(&input, "t".into());
    assert_eq!(render_statusline(&state), "[Opus 4.8 (1M context)] limits pending");
}
