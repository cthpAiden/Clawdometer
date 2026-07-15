use serde::{Deserialize, Deserializer, Serialize};

/// Fields we consume from Claude Code's statusline stdin JSON (CLI 2.1.205,
/// verified against real dumps 2026-07-12). Unknown fields are ignored so
/// future CLI additions never break parsing. Complex sub-objects are lenient:
/// a malformed one degrades to None instead of failing the whole input (a
/// total failure would kill the statusline AND stop state.json updates), and
/// percentages/timestamps accept floats by rounding.
#[derive(Debug, Clone, Deserialize)]
pub struct StatuslineInput {
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub model: Option<Model>,
    pub version: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub rate_limits: Option<RateLimits>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub context_window: Option<ContextWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RateLimits {
    #[serde(default, deserialize_with = "lenient_opt")]
    pub five_hour: Option<LimitWindow>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub seven_day: Option<LimitWindow>,
    /// The per-model weekly limit. Unlike the two above this never arrives on
    /// the statusline's stdin — only `/usage` prints it — so it is always None
    /// on the hook path and Some only after a refresh. state::merge carries it
    /// across snapshots for exactly that reason.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub fable_week: Option<LimitWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LimitWindow {
    #[serde(deserialize_with = "round_i64")]
    pub used_percentage: i64,
    #[serde(deserialize_with = "round_i64")]
    pub resets_at: i64,
}

/// Empirical: key present even pre-first-response, with null percentages.
#[derive(Debug, Clone, Deserialize)]
pub struct ContextWindow {
    #[serde(default, deserialize_with = "round_opt_i64")]
    pub used_percentage: Option<i64>,
}

/// Deserialize to None instead of erroring when the value has the wrong shape.
fn lenient_opt<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok()))
}

/// Accept integer or float, rounding to i64. f64 is exact past any epoch
/// second or percentage we'll ever see (2^53).
fn round_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(f64::deserialize(deserializer)?.round() as i64)
}

fn round_opt_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<f64>::deserialize(deserializer)?.map(|n| n.round() as i64))
}

pub fn parse_statusline_input(raw: &str) -> Result<StatuslineInput, serde_json::Error> {
    serde_json::from_str(raw.trim_start_matches('\u{feff}'))
}
