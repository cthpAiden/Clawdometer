use serde::{Deserialize, Serialize};

/// Fields we consume from Claude Code's statusline stdin JSON (CLI 2.1.205,
/// verified against real dumps 2026-07-12). Unknown fields are ignored so
/// future CLI additions never break parsing.
#[derive(Debug, Clone, Deserialize)]
pub struct StatuslineInput {
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub model: Option<Model>,
    pub version: Option<String>,
    pub rate_limits: Option<RateLimits>,
    pub context_window: Option<ContextWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RateLimits {
    pub five_hour: Option<LimitWindow>,
    pub seven_day: Option<LimitWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LimitWindow {
    pub used_percentage: i64,
    pub resets_at: i64,
}

/// Empirical: key present even pre-first-response, with null percentages.
#[derive(Debug, Clone, Deserialize)]
pub struct ContextWindow {
    pub used_percentage: Option<i64>,
}

pub fn parse_statusline_input(raw: &str) -> Result<StatuslineInput, serde_json::Error> {
    serde_json::from_str(raw.trim_start_matches('\u{feff}'))
}
