use clawdometer_core::schema::parse_statusline_input;
use clawdometer_core::state::{read_state, write_state_atomic, State};

const PRE: &str = include_str!("fixtures/stdin-pre-response.json");
const FULL: &str = include_str!("fixtures/stdin-with-limits.json");

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

use clawdometer_core::state::render_statusline;

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
