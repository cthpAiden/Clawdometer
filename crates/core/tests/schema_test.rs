use clawdometer_core::schema::parse_statusline_input;

const PRE: &str = include_str!("fixtures/stdin-pre-response.json");
const FULL: &str = include_str!("fixtures/stdin-with-limits.json");

#[test]
fn parses_pre_response_dump() {
    let input = parse_statusline_input(PRE).unwrap();
    assert!(input.rate_limits.is_none(), "pre-response dump has no rate_limits");
    // Empirical: context_window key IS present pre-response, with null percentages.
    assert_eq!(input.context_window.unwrap().used_percentage, None);
    let model = input.model.unwrap();
    assert_eq!(model.id, "claude-opus-4-8[1m]");
    assert_eq!(model.display_name, "Opus 4.8 (1M context)");
    assert_eq!(input.version.as_deref(), Some("2.1.205"));
    assert_eq!(
        input.session_id.as_deref(),
        Some("c3ef2f13-9695-476e-8bcf-0152b8d5c5d1")
    );
    assert!(input
        .transcript_path
        .as_deref()
        .unwrap()
        .ends_with("c3ef2f13-9695-476e-8bcf-0152b8d5c5d1.jsonl"));
}

#[test]
fn parses_full_dump_with_rate_limits() {
    let input = parse_statusline_input(FULL).unwrap();
    let rl = input.rate_limits.unwrap();
    let fh = rl.five_hour.unwrap();
    assert_eq!(fh.used_percentage, 1);
    assert_eq!(fh.resets_at, 1783814400);
    let sd = rl.seven_day.unwrap();
    assert_eq!(sd.used_percentage, 5);
    assert_eq!(sd.resets_at, 1784170800);
    assert_eq!(input.context_window.unwrap().used_percentage, Some(4));
}

#[test]
fn tolerates_bom_and_unknown_fields() {
    let raw = format!("\u{feff}{}", r#"{"model":{"id":"x","display_name":"X"},"brand_new_key":123}"#);
    let input = parse_statusline_input(&raw).unwrap();
    assert_eq!(input.model.unwrap().id, "x");
    assert!(input.rate_limits.is_none());
}

#[test]
fn rejects_garbage() {
    assert!(parse_statusline_input("not json at all").is_err());
    assert!(parse_statusline_input("").is_err());
}

// Robustness: a type drift or malformed sub-object in ONE field must degrade
// that field to None (or round it), never reject the whole input — a total
// parse failure kills the statusline AND stops state.json updates.

#[test]
fn tolerates_float_percentages_by_rounding() {
    let raw = r#"{
        "rate_limits": {
            "five_hour": {"used_percentage": 4.6, "resets_at": 1783814400},
            "seven_day": {"used_percentage": 5.4, "resets_at": 1784170800.9}
        },
        "context_window": {"used_percentage": 3.5}
    }"#;
    let input = parse_statusline_input(raw).unwrap();
    let rl = input.rate_limits.unwrap();
    assert_eq!(rl.five_hour.as_ref().unwrap().used_percentage, 5);
    assert_eq!(rl.seven_day.as_ref().unwrap().used_percentage, 5);
    assert_eq!(rl.seven_day.unwrap().resets_at, 1784170801);
    assert_eq!(input.context_window.unwrap().used_percentage, Some(4));
}

#[test]
fn malformed_model_degrades_to_none_not_total_failure() {
    // display_name missing: model unusable, but limits must still parse.
    let raw = r#"{
        "model": {"id": "claude-x"},
        "rate_limits": {"five_hour": {"used_percentage": 9, "resets_at": 1}, "seven_day": null}
    }"#;
    let input = parse_statusline_input(raw).unwrap();
    assert!(input.model.is_none());
    assert_eq!(input.rate_limits.unwrap().five_hour.unwrap().used_percentage, 9);
}

#[test]
fn malformed_rate_limits_degrade_to_none_not_total_failure() {
    let raw = r#"{
        "model": {"id": "x", "display_name": "X"},
        "rate_limits": "what even is this",
        "context_window": {"used_percentage": "also garbage"}
    }"#;
    let input = parse_statusline_input(raw).unwrap();
    assert!(input.rate_limits.is_none());
    assert!(input.context_window.is_none());
    assert_eq!(input.model.unwrap().display_name, "X");
}

#[test]
fn malformed_window_inside_rate_limits_degrades_only_that_window() {
    let raw = r#"{
        "rate_limits": {
            "five_hour": {"used_percentage": "garbage", "resets_at": 1},
            "seven_day": {"used_percentage": 5, "resets_at": 1784170800}
        }
    }"#;
    let input = parse_statusline_input(raw).unwrap();
    let rl = input.rate_limits.unwrap();
    assert!(rl.five_hour.is_none());
    assert_eq!(rl.seven_day.unwrap().used_percentage, 5);
}
