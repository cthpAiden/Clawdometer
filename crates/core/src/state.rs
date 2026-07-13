use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::schema::{Model, RateLimits, StatuslineInput};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub schema_version: u32,
    pub captured_at: String,
    pub rate_limits: Option<RateLimits>,
    pub model: Option<Model>,
    pub context_window: Option<StateContextWindow>,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub cli_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateContextWindow {
    pub used_percentage: i64,
}

impl State {
    pub fn from_input(input: &StatuslineInput, captured_at: String) -> State {
        State {
            schema_version: SCHEMA_VERSION,
            captured_at,
            rate_limits: input.rate_limits.clone(),
            model: input.model.clone(),
            context_window: input
                .context_window
                .as_ref()
                .and_then(|cw| cw.used_percentage)
                .map(|used_percentage| StateContextWindow { used_percentage }),
            session_id: input.session_id.clone(),
            transcript_path: input.transcript_path.clone(),
            cli_version: input.version.clone(),
        }
    }
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .replace_millisecond(0)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}

/// Temp file in the same dir + atomic rename. Last-write-wins across
/// concurrent sessions is correct (limits are account-wide).
pub fn write_state_atomic(path: &Path, state: &State) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "state path has no parent"))?;
    std::fs::create_dir_all(dir)?;
    let body = serde_json::to_string_pretty(state)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(body.as_bytes())?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// None on missing/malformed/torn file. Readers retry next cycle.
pub fn read_state(path: &Path) -> Option<State> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()
}

/// Merge the statusline snapshot (state.json) with the refresh snapshot
/// (live.json, from a headless `claude /usage` run). rate_limits +
/// captured_at come from whichever snapshot is newer AND actually has
/// rate_limits; model/context only ever exist in the statusline snapshot.
/// captured_at strings are RFC3339 UTC from the same formatter, so
/// lexicographic comparison is chronological.
pub fn merge(state: Option<State>, live: Option<State>) -> Option<State> {
    match (state, live) {
        (Some(s), Some(l)) => {
            let use_live = l.rate_limits.is_some()
                && (s.rate_limits.is_none() || l.captured_at > s.captured_at);
            if use_live {
                Some(State { rate_limits: l.rate_limits, captured_at: l.captured_at, ..s })
            } else {
                Some(s)
            }
        }
        (s, l) => s.or(l),
    }
}

/// A window whose reset time has passed no longer exists server-side: until
/// the next request opens a new one, true usage is 0%. Data only arrives via
/// the statusline hook and periodic /usage refreshes, so an idle machine
/// would otherwise show the last snapshot's percentage forever. Zeroes such
/// windows in place; resets_at is kept so the UI can label the value as
/// post-reset.
pub fn zero_expired_windows(rate_limits: &mut RateLimits, now_epoch_secs: i64) {
    for w in [&mut rate_limits.five_hour, &mut rate_limits.seven_day] {
        if let Some(w) = w.as_mut() {
            if w.resets_at <= now_epoch_secs {
                w.used_percentage = 0;
            }
        }
    }
}

/// One-line statusline text. Absent rate_limits is a normal state.
pub fn render_statusline(state: &State) -> String {
    let model = state
        .model
        .as_ref()
        .map(|m| m.display_name.as_str())
        .unwrap_or("Claude");
    match &state.rate_limits {
        Some(rl) => {
            let pct = |w: &Option<crate::schema::LimitWindow>| {
                w.as_ref()
                    .map(|w| format!("{}%", w.used_percentage))
                    .unwrap_or_else(|| "?".into())
            };
            format!("[{model}] 5h {} · 7d {}", pct(&rl.five_hour), pct(&rl.seven_day))
        }
        None => format!("[{model}] limits pending"),
    }
}
