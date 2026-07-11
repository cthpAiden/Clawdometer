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
