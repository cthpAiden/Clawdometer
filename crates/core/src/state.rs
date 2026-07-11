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
