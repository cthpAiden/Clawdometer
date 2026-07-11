# Clawdometer v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Clawdometer v1 — a Windows desktop HUD that shows Claude Code rate-limit usage from local statusline data, plus a safe installer/uninstaller for the statusline hook.

**Architecture:** Rust workspace. `crates/core` is a pure library (stdin schema parsing, atomic `state.json` IO, settings.json install/uninstall merge). `crates/cli` builds `clawdometer.exe` with `hook | install | uninstall | status` subcommands. `app/src-tauri` is a thin Tauri v2 host that watches `state.json` and pushes a `StateUpdated` event to the webview skin in `app/ui`.

**Tech Stack:** Rust 2021, serde/serde_json, tempfile (atomic renames), time (timestamps), dirs (home dir), wait-timeout (wrapped-command timeout), notify (file watch), Tauri v2 + plain HTML/CSS/JS (no npm build), cargo-deny (network-crate ban).

**Spec:** `docs/superpowers/specs/2026-07-12-clawdometer-design.md`. Schemas in the spec are **empirical — do not re-derive them**. Real fixtures: `~/.clawdometer/probe/stdin-20260712-020203-302.json` (pre-first-response) and `~/.clawdometer/probe/stdin-20260712-020216-011.json` (with `rate_limits`).

**Fixture-over-spec note:** The spec prose says the pre-response dump has "NO `context_window` key". The actual captured dump HAS a `context_window` key with `"used_percentage": null`. The fixtures are ground truth: parse `context_window` as `Option<ContextWindow>` with `used_percentage: Option<i64>`, and normalize a null percentage to `context_window: null` in `state.json`.

## Global Constraints

Copied from the spec — every task implicitly includes these:

1. **NO network calls anywhere in the workspace.** Enforced via cargo-deny ban on network crates + CI check.
2. **NO reads of `~/.claude/.credentials.json`** or any credential store.
3. **NO impersonation of the Claude Code CLI.**
4. **Writes ONLY under `~/.clawdometer/`** + the `statusLine` key of settings.json during install/uninstall. (Known tension: the spec's tray menu includes "Start with Windows", which requires an HKCU Run-key write. Resolution in Task 14: the write happens only on an explicit user toggle, and the README documents it as the single exception.)
5. All `~/.claude` reads opened **read-only**.
6. Hook must **NEVER break the user's statusline**: always emit one line of text, always exit 0, <100ms typical.
7. `used_percentage` is an **integer**; `resets_at` is **unix epoch seconds**. `rate_limits` absent is a **normal state**, not an error.
8. `state.json` is `schema_version: 1`, written via temp-file-in-same-dir + atomic rename. Last-write-wins across concurrent sessions is correct.
9. HUD: no error dialogs, ever. Missing/malformed/torn state → "waiting" state. `resets_at` in the past → "refresh pending", never a negative countdown.
10. Installer: parse → modify **only** `statusLine` → serialize → atomic rename. All other keys survive semantically intact (deep-equal test). Backups are never overwritten. Malformed settings.json → abort, touch nothing.
11. Testing overrides: `CLAWDOMETER_DIR` env var overrides `~/.clawdometer`; settings path is always passed as a parameter/flag so tests never touch the real `~/.claude/settings.json`.

---

## Phase 1 — Workspace scaffold + core crate

**Milestone success criteria (verify all before Phase 2):**
- `cargo build --workspace` and `cargo test --workspace` pass.
- `cargo deny check bans` passes with the network-crate ban list active.
- `crates/core/tests/fixtures/` contains byte-exact copies of the two probe dumps, and parsing tests against both pass.
- `write_state_atomic` round-trip test passes (write → read → equal), including overwrite of an existing file.

### Task 1: Workspace scaffold + cargo-deny network ban

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/core/Cargo.toml`, `crates/core/src/lib.rs`
- Create: `deny.toml`
- Create: `.gitignore`

**Interfaces:**
- Produces: workspace layout all later tasks build in; `clawdometer-core` crate name.

- [ ] **Step 1: Create workspace root `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = ["crates/core"]
```

(`crates/cli` is added in Task 4, `app/src-tauri` in Task 11.)

- [ ] **Step 2: Create `crates/core/Cargo.toml`**

```toml
[package]
name = "clawdometer-core"
version = "0.1.0"
edition = "2021"
license = "MIT"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tempfile = "3"
time = { version = "0.3", features = ["formatting", "parsing"] }

[dev-dependencies]
```

- [ ] **Step 3: Create `crates/core/src/lib.rs`**

```rust
pub mod paths;
pub mod schema;
pub mod settings;
pub mod state;
```

Create the four module files as empty files for now (`paths.rs`, `schema.rs`, `settings.rs`, `state.rs`) so the crate compiles.

- [ ] **Step 4: Create `.gitignore`**

```
/target
```

- [ ] **Step 5: Create `deny.toml` with the network-crate ban**

```toml
# Safety invariant 1: NO network calls anywhere in the workspace.
# Any crate capable of opening a socket to the internet is banned.
[bans]
multiple-versions = "allow"
deny = [
    { crate = "reqwest" },
    { crate = "hyper" },
    { crate = "hyper-util" },
    { crate = "ureq" },
    { crate = "curl" },
    { crate = "curl-sys" },
    { crate = "isahc" },
    { crate = "attohttpc" },
    { crate = "surf" },
    { crate = "native-tls" },
    { crate = "openssl" },
    { crate = "rustls" },
    { crate = "tungstenite" },
    { crate = "tokio-tungstenite" },
]

[licenses]
allow = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-3.0", "Zlib", "MPL-2.0", "CDLA-Permissive-2.0", "BSL-1.0"]

[advisories]
```

Note for Task 11: if Tauri transitively pulls a banned crate, add a `wrappers = ["<tauri crate>"]` exception on that specific ban entry with a comment explaining the transitive path — never add exceptions for `clawdometer-core` or `clawdometer-cli` dependencies. If cargo-deny complains about the `[licenses]` allow list on real dependencies, extend the list with the specific license — do not disable license checking.

- [ ] **Step 6: Verify build and ban check**

Run: `cargo build --workspace`
Expected: compiles clean.

Run: `cargo install cargo-deny --locked` (if not installed), then `cargo deny check bans`
Expected: `bans ok`.

Sanity-check the ban actually fires: temporarily add `ureq = "2"` to `crates/core/Cargo.toml` dependencies, run `cargo deny check bans`, expect FAILURE naming `ureq`. Remove it again and re-verify `bans ok`.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/core deny.toml .gitignore
git commit -m "feat: scaffold Rust workspace with core crate and cargo-deny network ban"
```

### Task 2: Statusline stdin schema (parse both real fixtures)

**Files:**
- Create: `crates/core/tests/fixtures/stdin-pre-response.json` (byte-exact copy of `~/.clawdometer/probe/stdin-20260712-020203-302.json`)
- Create: `crates/core/tests/fixtures/stdin-with-limits.json` (byte-exact copy of `~/.clawdometer/probe/stdin-20260712-020216-011.json`)
- Modify: `crates/core/src/schema.rs`
- Test: `crates/core/tests/schema_test.rs`

**Interfaces:**
- Produces:
  - `schema::StatuslineInput { session_id: Option<String>, transcript_path: Option<String>, model: Option<Model>, version: Option<String>, rate_limits: Option<RateLimits>, context_window: Option<ContextWindow> }`
  - `schema::Model { id: String, display_name: String }`
  - `schema::RateLimits { five_hour: Option<LimitWindow>, seven_day: Option<LimitWindow> }`
  - `schema::LimitWindow { used_percentage: i64, resets_at: i64 }`
  - `schema::ContextWindow { used_percentage: Option<i64> }`
  - `schema::parse_statusline_input(raw: &str) -> Result<StatuslineInput, serde_json::Error>`

- [ ] **Step 1: Copy the two probe dumps into fixtures**

```bash
mkdir -p crates/core/tests/fixtures
cp ~/.clawdometer/probe/stdin-20260712-020203-302.json crates/core/tests/fixtures/stdin-pre-response.json
cp ~/.clawdometer/probe/stdin-20260712-020216-011.json crates/core/tests/fixtures/stdin-with-limits.json
```

Do NOT edit the copies. They are the empirical schema.

- [ ] **Step 2: Write failing tests** — `crates/core/tests/schema_test.rs`

```rust
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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p clawdometer-core --test schema_test`
Expected: FAIL — `parse_statusline_input` not found.

- [ ] **Step 4: Implement `crates/core/src/schema.rs`**

```rust
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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p clawdometer-core --test schema_test`
Expected: 4 passed.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/schema.rs crates/core/tests
git commit -m "feat: parse statusline stdin schema against real captured fixtures"
```

### Task 3: state.json model + atomic write + paths module

**Files:**
- Modify: `crates/core/src/state.rs`
- Modify: `crates/core/src/paths.rs`
- Modify: `crates/core/Cargo.toml` (add `dirs = "6"` dependency)
- Test: `crates/core/tests/state_test.rs`

**Interfaces:**
- Consumes: `schema::{StatuslineInput, Model, RateLimits}` from Task 2.
- Produces:
  - `state::State { schema_version: u32, captured_at: String, rate_limits: Option<RateLimits>, model: Option<Model>, context_window: Option<StateContextWindow>, session_id: Option<String>, transcript_path: Option<String>, cli_version: Option<String> }`
  - `state::StateContextWindow { used_percentage: i64 }`
  - `state::State::from_input(input: &StatuslineInput, captured_at: String) -> State`
  - `state::write_state_atomic(path: &Path, state: &State) -> std::io::Result<()>`
  - `state::read_state(path: &Path) -> Option<State>` (None on missing/malformed/torn — never errors)
  - `state::now_rfc3339() -> String`
  - `paths::clawdometer_dir() -> PathBuf` (respects `CLAWDOMETER_DIR` env override)
  - `paths::state_path() -> PathBuf`, `paths::wrapped_path() -> PathBuf`

- [ ] **Step 1: Add `dirs` dependency**

In `crates/core/Cargo.toml` `[dependencies]` add:

```toml
dirs = "6"
```

- [ ] **Step 2: Write failing tests** — `crates/core/tests/state_test.rs`

```rust
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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p clawdometer-core --test state_test`
Expected: FAIL — `State` not found.

- [ ] **Step 4: Implement `crates/core/src/state.rs`**

```rust
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
```

- [ ] **Step 5: Implement `crates/core/src/paths.rs`**

```rust
use std::path::PathBuf;

/// ~/.clawdometer, overridable via CLAWDOMETER_DIR (used by tests; never
/// point it at a real ~/.claude directory).
pub fn clawdometer_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAWDOMETER_DIR") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".clawdometer")
}

pub fn state_path() -> PathBuf {
    clawdometer_dir().join("state.json")
}

pub fn wrapped_path() -> PathBuf {
    clawdometer_dir().join("wrapped.json")
}

pub fn default_claude_settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("settings.json")
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p clawdometer-core`
Expected: all schema + state tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/core
git commit -m "feat: state.json model with atomic write and tolerant read"
```

---

## Phase 2 — `hook` subcommand

**Milestone success criteria (verify all before Phase 3):**
- `echo <real dump> | clawdometer hook` writes a spec-shaped `state.json`, prints one statusline line, exits 0 — verified by integration tests piping both fixture files into the built exe.
- Garbage stdin, empty stdin, and an unwritable `CLAWDOMETER_DIR` all still print a line and exit 0.
- With `wrapped.json` present, the hook pipes stdin to the wrapped command and passes its stdout through; a hanging wrapped command times out at 2s and falls back to our own line.
- `Measure-Command { Get-Content fixture | clawdometer.exe hook }` (release build) completes well under 100ms.

### Task 4: CLI crate + hook happy path

**Files:**
- Create: `crates/cli/Cargo.toml`, `crates/cli/src/main.rs`, `crates/cli/src/hook.rs`
- Modify: `Cargo.toml` (workspace members)
- Modify: `crates/core/src/state.rs` (add `render_statusline`)
- Test: `crates/core/tests/state_test.rs` (render tests), `crates/cli/tests/hook_test.rs`

**Interfaces:**
- Consumes: `schema::parse_statusline_input`, `state::{State, write_state_atomic, now_rfc3339}`, `paths::{state_path, wrapped_path}`.
- Produces:
  - `state::render_statusline(state: &State) -> String`
  - `clawdometer.exe hook` behavior: reads all of stdin, writes `paths::state_path()`, prints one line, exits 0.
  - `hook::run_hook() -> String` (infallible; returns the line to print)

- [ ] **Step 1: Add cli crate to workspace**

Workspace root `Cargo.toml` members become:

```toml
members = ["crates/core", "crates/cli"]
```

`crates/cli/Cargo.toml`:

```toml
[package]
name = "clawdometer-cli"
version = "0.1.0"
edition = "2021"
license = "MIT"

[[bin]]
name = "clawdometer"
path = "src/main.rs"

[dependencies]
clawdometer-core = { path = "../core" }
serde_json = "1"
wait-timeout = "0.2"
time = { version = "0.3", features = ["formatting"] }

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Write failing render tests** — append to `crates/core/tests/state_test.rs`

```rust
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
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p clawdometer-core --test state_test`
Expected: FAIL — `render_statusline` not found.

- [ ] **Step 4: Implement `render_statusline`** — append to `crates/core/src/state.rs`

```rust
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
```

Run: `cargo test -p clawdometer-core` — expected PASS.

- [ ] **Step 5: Write failing integration test** — `crates/cli/tests/hook_test.rs`

```rust
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const PRE: &str = include_str!("../../core/tests/fixtures/stdin-pre-response.json");
const FULL: &str = include_str!("../../core/tests/fixtures/stdin-with-limits.json");

fn run_hook(stdin: &str, clawdometer_dir: &Path) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_clawdometer"))
        .arg("hook")
        .env("CLAWDOMETER_DIR", clawdometer_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (String::from_utf8_lossy(&out.stdout).trim().to_string(), out.status.code().unwrap())
}

#[test]
fn hook_writes_state_and_prints_line() {
    let dir = tempfile::tempdir().unwrap();
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%");
    let state = clawdometer_core::state::read_state(&dir.path().join("state.json")).unwrap();
    assert_eq!(state.schema_version, 1);
    assert_eq!(state.rate_limits.unwrap().seven_day.unwrap().resets_at, 1784170800);
}

#[test]
fn hook_pre_response_writes_null_limits() {
    let dir = tempfile::tempdir().unwrap();
    let (line, code) = run_hook(PRE, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] limits pending");
    let state = clawdometer_core::state::read_state(&dir.path().join("state.json")).unwrap();
    assert!(state.rate_limits.is_none());
}
```

Add `clawdometer-core` is already a dependency; tests use it directly.

- [ ] **Step 6: Run to verify failure**

Run: `cargo test -p clawdometer-cli --test hook_test`
Expected: FAIL (binary has no hook logic yet / doesn't compile).

- [ ] **Step 7: Implement `crates/cli/src/main.rs`**

```rust
mod hook;

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    match arg.as_str() {
        "hook" => {
            // Safety invariant: NEVER break the user's statusline.
            let line = std::panic::catch_unwind(hook::run_hook)
                .unwrap_or_else(|_| String::from("clawdometer"));
            println!("{line}");
            std::process::exit(0);
        }
        "install" | "uninstall" | "status" => {
            eprintln!("{arg}: not implemented yet");
            std::process::exit(2);
        }
        _ => {
            eprintln!("usage: clawdometer <hook|install|uninstall|status>");
            std::process::exit(2);
        }
    }
}
```

- [ ] **Step 8: Implement `crates/cli/src/hook.rs` (happy path; wrapping added in Task 6)**

```rust
use std::io::Read;

use clawdometer_core::paths;
use clawdometer_core::schema::parse_statusline_input;
use clawdometer_core::state::{now_rfc3339, render_statusline, write_state_atomic, State};

const FALLBACK_LINE: &str = "clawdometer: waiting";

/// Infallible: every failure path still returns a printable line.
pub fn run_hook() -> String {
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return FALLBACK_LINE.into();
    }
    let input = match parse_statusline_input(&raw) {
        Ok(input) => input,
        Err(_) => return FALLBACK_LINE.into(),
    };
    let state = State::from_input(&input, now_rfc3339());
    // Write failure must not break the statusline; HUD just stays stale.
    let _ = write_state_atomic(&paths::state_path(), &state);
    render_statusline(&state)
}
```

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test -p clawdometer-cli --test hook_test`
Expected: 2 passed.

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml crates/cli crates/core
git commit -m "feat: clawdometer hook subcommand writes state.json and prints statusline"
```

### Task 5: Hook error hardening (garbage stdin, unwritable dir)

**Files:**
- Test: `crates/cli/tests/hook_test.rs` (append)
- Modify: `crates/cli/src/hook.rs` (only if a test exposes a gap)

**Interfaces:**
- Consumes: `run_hook` from Task 4. No new interfaces.

- [ ] **Step 1: Write failing/verifying tests** — append to `crates/cli/tests/hook_test.rs`

```rust
#[test]
fn hook_survives_garbage_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let (line, code) = run_hook("%%% not json {{{", dir.path());
    assert_eq!(code, 0);
    assert!(!line.is_empty());
    assert!(!dir.path().join("state.json").exists(), "garbage must not produce a state file");
}

#[test]
fn hook_survives_empty_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let (line, code) = run_hook("", dir.path());
    assert_eq!(code, 0);
    assert!(!line.is_empty());
}

#[test]
fn hook_survives_unwritable_state_dir() {
    // CLAWDOMETER_DIR whose parent is a FILE -> create_dir_all fails.
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "i am a file").unwrap();
    let bad_dir = blocker.join("nested");
    let (line, code) = run_hook(FULL, &bad_dir);
    assert_eq!(code, 0, "unwritable dir must still exit 0");
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%", "statusline still renders from parsed input");
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p clawdometer-cli --test hook_test`
Expected: all pass with the Task 4 implementation (it was written fail-safe). If any fail, fix `hook.rs` minimally until green — do not weaken assertions.

- [ ] **Step 3: Measure startup budget (release)**

```powershell
cargo build --release -p clawdometer-cli
Measure-Command { Get-Content crates/core/tests/fixtures/stdin-with-limits.json | .\target\release\clawdometer.exe hook }
```

Expected: TotalMilliseconds well under 100. Record the number in the commit message.

- [ ] **Step 4: Commit**

```bash
git add crates/cli
git commit -m "test: hook error-injection coverage (garbage stdin, unwritable dir)"
```

### Task 6: Wrapped-statusline chaining with 2s timeout

**Files:**
- Modify: `crates/cli/src/hook.rs`
- Test: `crates/cli/tests/hook_test.rs` (append)

**Interfaces:**
- Consumes: `paths::wrapped_path()`; `wrapped.json` = the user's FULL original `statusLine` object (written by the installer in Task 7), e.g. `{"command": "...", "padding": 0}`.
- Produces: hook behavior — if `wrapped.json` exists and its `command` runs successfully within 2s, the hook prints THAT command's first stdout line; otherwise falls back to our own line.

- [ ] **Step 1: Write failing tests** — append to `crates/cli/tests/hook_test.rs`

```rust
fn write_wrapped(dir: &Path, command: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let obj = serde_json::json!({ "command": command, "padding": 0 });
    std::fs::write(dir.join("wrapped.json"), serde_json::to_string(&obj).unwrap()).unwrap();
}

#[test]
fn hook_passes_through_wrapped_command_output() {
    let dir = tempfile::tempdir().unwrap();
    write_wrapped(dir.path(), "echo original-statusline");
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "original-statusline");
    // state.json still written even when passing through
    assert!(clawdometer_core::state::read_state(&dir.path().join("state.json")).is_some());
}

#[test]
fn hook_falls_back_when_wrapped_command_fails() {
    let dir = tempfile::tempdir().unwrap();
    write_wrapped(dir.path(), "cmd /C exit 3");
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%");
}

#[test]
fn hook_falls_back_when_wrapped_command_hangs() {
    let dir = tempfile::tempdir().unwrap();
    // ping -n 30 sleeps ~29s on Windows; must be killed at the 2s timeout.
    write_wrapped(dir.path(), "ping -n 30 127.0.0.1");
    let start = std::time::Instant::now();
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%");
    assert!(start.elapsed() < std::time::Duration::from_secs(10), "timeout did not fire");
}

#[test]
fn hook_falls_back_when_wrapped_json_malformed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path()).unwrap();
    std::fs::write(dir.path().join("wrapped.json"), "{ nope").unwrap();
    let (line, code) = run_hook(FULL, dir.path());
    assert_eq!(code, 0);
    assert_eq!(line, "[Opus 4.8 (1M context)] 5h 1% · 7d 5%");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p clawdometer-cli --test hook_test`
Expected: the new pass-through test FAILS (hook prints its own line, not `original-statusline`).

- [ ] **Step 3: Implement chaining in `crates/cli/src/hook.rs`**

Replace the final line of `run_hook` and add the helper:

```rust
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

use wait_timeout::ChildExt;
```

In `run_hook`, replace `render_statusline(&state)` with:

```rust
    if let Some(line) = run_wrapped(&paths::wrapped_path(), &raw) {
        return line;
    }
    render_statusline(&state)
```

Helper (same file):

```rust
/// Chain the user's original statusline command. Any failure -> None,
/// caller falls back to our own line. 2s hard timeout.
fn run_wrapped(wrapped_path: &std::path::Path, stdin_raw: &str) -> Option<String> {
    let raw = std::fs::read_to_string(wrapped_path).ok()?;
    let value: serde_json::Value =
        serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()?;
    let command = value.get("command")?.as_str()?.to_string();

    let mut cmd = shell_command(&command);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Ignore write errors: the child may exit without reading stdin.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_raw.as_bytes());
    }
    match child.wait_timeout(Duration::from_secs(2)) {
        Ok(Some(status)) if status.success() => {
            let mut out = String::new();
            use std::io::Read as _;
            child.stdout.take()?.read_to_string(&mut out).ok()?;
            let line = out.lines().next()?.trim().to_string();
            if line.is_empty() { None } else { Some(line) }
        }
        Ok(Some(_)) => None,
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new("cmd");
    // raw_arg: hand the command string to cmd.exe unmangled.
    cmd.arg("/C").raw_arg(command);
    cmd
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clawdometer-cli --test hook_test`
Expected: all pass, including the hang test finishing in ~2s.

- [ ] **Step 5: Commit**

```bash
git add crates/cli
git commit -m "feat: hook chains wrapped statusline command with 2s timeout fallback"
```

---

## Phase 3 — Installer / uninstaller (riskiest code — full spec test matrix)

**Milestone success criteria (verify all before Phase 4):**
- Every row of the spec's installer test matrix has a passing test: missing settings.json · empty `{}` · existing statusLine (wrapped) · statusLine with extra fields (round-trip preserved) · BOM/CRLF · unicode values · install twice (idempotent) · uninstall without install · uninstall after manual user edit (warn, touch nothing, non-zero) · malformed JSON (abort untouched).
- Deep-equal test proves all non-`statusLine` keys survive install and uninstall semantically intact.
- Backups: created before every mutating install, never overwritten (second install with same timestamp gets a distinct filename).
- `clawdometer status` prints a state summary; `--purge` removes `~/.clawdometer`.

### Task 7: Core install logic (basic matrix rows)

**Files:**
- Modify: `crates/core/src/settings.rs`
- Test: `crates/core/tests/settings_install_test.rs`

**Interfaces:**
- Produces:
  - `settings::InstallOutcome { Installed, Wrapped, AlreadyInstalled }` (derive `Debug, PartialEq`)
  - `settings::SettingsError { MalformedSettings(String), Io(std::io::Error) }` (derive `Debug`, impl `Display`)
  - `settings::install(settings_path: &Path, clawdometer_dir: &Path, our_command: &str, timestamp: &str) -> Result<InstallOutcome, SettingsError>`
  - `wrapped.json` written under `clawdometer_dir` = full original `statusLine` JSON object (consumed by Task 6's hook and Task 9's uninstall)
  - Backups at `<clawdometer_dir>/backups/settings-<timestamp>.json` (raw original bytes)

- [ ] **Step 1: Write failing tests** — `crates/core/tests/settings_install_test.rs`

```rust
use std::path::{Path, PathBuf};

use clawdometer_core::settings::{install, InstallOutcome, SettingsError};

const OURS: &str = r#""C:\bin\clawdometer.exe" hook"#;

struct Env {
    _tmp: tempfile::TempDir,
    settings: PathBuf,
    claw: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("clawdometer");
    Env { settings, claw, _tmp: tmp }
}

fn read_json(path: &Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(raw.trim_start_matches('\u{feff}')).unwrap()
}

#[test]
fn install_with_missing_settings_creates_file() {
    let e = env();
    let outcome = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    assert_eq!(outcome, InstallOutcome::Installed);
    let json = read_json(&e.settings);
    assert_eq!(json["statusLine"]["command"], OURS);
    assert!(!e.claw.join("wrapped.json").exists());
    // no backup for a file that didn't exist
    assert!(!e.claw.join("backups").exists());
}

#[test]
fn install_with_empty_object() {
    let e = env();
    std::fs::write(&e.settings, "{}").unwrap();
    let outcome = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    assert_eq!(outcome, InstallOutcome::Installed);
    assert_eq!(read_json(&e.settings)["statusLine"]["command"], OURS);
}

#[test]
fn install_wraps_existing_statusline_preserving_extra_fields() {
    let e = env();
    std::fs::write(
        &e.settings,
        r#"{"statusLine":{"command":"my-old-line.cmd","padding":0,"custom":true},"model":"opus"}"#,
    ).unwrap();
    let outcome = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    assert_eq!(outcome, InstallOutcome::Wrapped);
    // full original object persisted, extra fields intact
    let wrapped = read_json(&e.claw.join("wrapped.json"));
    assert_eq!(wrapped["command"], "my-old-line.cmd");
    assert_eq!(wrapped["padding"], 0);
    assert_eq!(wrapped["custom"], true);
    // statusLine replaced, other keys survive
    let json = read_json(&e.settings);
    assert_eq!(json["statusLine"]["command"], OURS);
    assert_eq!(json["model"], "opus");
}

#[test]
fn install_preserves_all_other_keys_deep_equal() {
    let e = env();
    let original = r#"{
        "model": "opus",
        "permissions": {"allow": ["Bash(ls:*)"], "deny": []},
        "env": {"FOO": "bar"},
        "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "x"}]}]}
    }"#;
    std::fs::write(&e.settings, original).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let mut before: serde_json::Value = serde_json::from_str(original).unwrap();
    let mut after = read_json(&e.settings);
    before.as_object_mut().unwrap().remove("statusLine");
    after.as_object_mut().unwrap().remove("statusLine");
    assert_eq!(before, after, "non-statusLine keys must survive semantically intact");
}

#[test]
fn install_backs_up_existing_file_and_never_overwrites_backups() {
    let e = env();
    std::fs::write(&e.settings, r#"{"a":1}"#).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let b1 = e.claw.join("backups").join("settings-20260712-000000.json");
    assert_eq!(std::fs::read_to_string(&b1).unwrap(), r#"{"a":1}"#, "backup is raw original bytes");

    // force a second mutating install with the SAME timestamp
    std::fs::write(&e.settings, r#"{"a":2}"#).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let backups: Vec<_> = std::fs::read_dir(e.claw.join("backups")).unwrap().collect();
    assert_eq!(backups.len(), 2, "second backup must get a distinct name, never overwrite");
    assert_eq!(std::fs::read_to_string(&b1).unwrap(), r#"{"a":1}"#, "first backup untouched");
}

#[test]
fn install_twice_is_idempotent() {
    let e = env();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let after_first = std::fs::read_to_string(&e.settings).unwrap();
    let outcome = install(&e.settings, &e.claw, OURS, "20260712-000001").unwrap();
    assert_eq!(outcome, InstallOutcome::AlreadyInstalled);
    assert_eq!(std::fs::read_to_string(&e.settings).unwrap(), after_first, "no-op leaves file untouched");
    assert!(!e.claw.join("wrapped.json").exists(), "must not wrap our own command");
}

#[test]
fn install_aborts_on_malformed_json_touching_nothing() {
    let e = env();
    std::fs::write(&e.settings, "{ this is not json").unwrap();
    let err = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap_err();
    assert!(matches!(err, SettingsError::MalformedSettings(_)));
    assert_eq!(std::fs::read_to_string(&e.settings).unwrap(), "{ this is not json");
    assert!(!e.claw.exists(), "abort must touch nothing, not even backups");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p clawdometer-core --test settings_install_test`
Expected: FAIL — `settings::install` not found.

- [ ] **Step 3: Implement `crates/core/src/settings.rs`**

```rust
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub const STATUSLINE_KEY: &str = "statusLine";

#[derive(Debug, PartialEq)]
pub enum InstallOutcome {
    Installed,
    Wrapped,
    AlreadyInstalled,
}

#[derive(Debug)]
pub enum SettingsError {
    MalformedSettings(String),
    Io(std::io::Error),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::MalformedSettings(msg) => {
                write!(f, "settings.json is not valid JSON — nothing was changed: {msg}")
            }
            SettingsError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl From<std::io::Error> for SettingsError {
    fn from(e: std::io::Error) -> Self {
        SettingsError::Io(e)
    }
}

/// (parsed root object, file existed, raw original bytes)
fn load_settings(path: &Path) -> Result<(Value, bool, Vec<u8>), SettingsError> {
    if !path.exists() {
        return Ok((serde_json::json!({}), false, Vec::new()));
    }
    let raw = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&raw);
    let root: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| SettingsError::MalformedSettings(e.to_string()))?;
    if !root.is_object() {
        return Err(SettingsError::MalformedSettings("root is not a JSON object".into()));
    }
    Ok((root, true, raw))
}

fn atomic_write(path: &Path, body: &[u8]) -> Result<(), SettingsError> {
    let dir = path.parent().ok_or_else(|| {
        SettingsError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "no parent dir"))
    })?;
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(body)?;
    tmp.persist(path).map_err(|e| SettingsError::Io(e.error))?;
    Ok(())
}

fn save_settings(path: &Path, root: &Value) -> Result<(), SettingsError> {
    let body = serde_json::to_string_pretty(root)
        .map_err(|e| SettingsError::MalformedSettings(e.to_string()))?;
    atomic_write(path, body.as_bytes())
}

/// Raw-bytes backup; never overwrites an existing backup.
fn backup(clawdometer_dir: &Path, timestamp: &str, raw: &[u8]) -> Result<PathBuf, SettingsError> {
    let dir = clawdometer_dir.join("backups");
    std::fs::create_dir_all(&dir)?;
    let mut candidate = dir.join(format!("settings-{timestamp}.json"));
    let mut n = 1;
    while candidate.exists() {
        candidate = dir.join(format!("settings-{timestamp}-{n}.json"));
        n += 1;
    }
    std::fs::write(&candidate, raw)?;
    Ok(candidate)
}

fn is_ours(status_line: &Value, our_command: &str) -> bool {
    status_line.get("command").and_then(|c| c.as_str()) == Some(our_command)
}

pub fn install(
    settings_path: &Path,
    clawdometer_dir: &Path,
    our_command: &str,
    timestamp: &str,
) -> Result<InstallOutcome, SettingsError> {
    let (mut root, existed, raw) = load_settings(settings_path)?;

    if let Some(existing) = root.get(STATUSLINE_KEY) {
        if is_ours(existing, our_command) {
            return Ok(InstallOutcome::AlreadyInstalled);
        }
        // Persist FULL original statusLine object (command + extra fields).
        let existing = existing.clone();
        if existed {
            backup(clawdometer_dir, timestamp, &raw)?;
        }
        std::fs::create_dir_all(clawdometer_dir)?;
        let wrapped_body = serde_json::to_string_pretty(&existing)
            .map_err(|e| SettingsError::MalformedSettings(e.to_string()))?;
        atomic_write(&clawdometer_dir.join("wrapped.json"), wrapped_body.as_bytes())?;
        root[STATUSLINE_KEY] = serde_json::json!({ "command": our_command });
        save_settings(settings_path, &root)?;
        return Ok(InstallOutcome::Wrapped);
    }

    if existed {
        backup(clawdometer_dir, timestamp, &raw)?;
    }
    root[STATUSLINE_KEY] = serde_json::json!({ "command": our_command });
    save_settings(settings_path, &root)?;
    Ok(InstallOutcome::Installed)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clawdometer-core --test settings_install_test`
Expected: 7 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat: settings.json install with wrap, backup, idempotency, malformed-abort"
```

### Task 8: Install matrix rows — BOM/CRLF, unicode, round-trip

**Files:**
- Test: `crates/core/tests/settings_install_test.rs` (append)
- Modify: `crates/core/src/settings.rs` (only if a test exposes a gap)

**Interfaces:**
- Consumes: `settings::install` from Task 7. No new interfaces.

- [ ] **Step 1: Write the remaining matrix tests** — append to `crates/core/tests/settings_install_test.rs`

```rust
#[test]
fn install_handles_bom_and_crlf() {
    let e = env();
    let content = "\u{feff}{\r\n  \"model\": \"opus\",\r\n  \"statusLine\": {\"command\": \"old.cmd\"}\r\n}\r\n";
    std::fs::write(&e.settings, content).unwrap();
    let outcome = install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    assert_eq!(outcome, InstallOutcome::Wrapped);
    let json = read_json(&e.settings);
    assert_eq!(json["model"], "opus");
    assert_eq!(json["statusLine"]["command"], OURS);
    // backup preserved the original bytes exactly, BOM and CRLF included
    let backup = std::fs::read(e.claw.join("backups").join("settings-20260712-000000.json")).unwrap();
    assert_eq!(backup, content.as_bytes());
}

#[test]
fn install_preserves_unicode_values() {
    let e = env();
    std::fs::write(
        &e.settings,
        r#"{"userName":"Phúc Châu 🦀","note":"日本語テスト","statusLine":{"command":"echo héllo"}}"#,
    ).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let json = read_json(&e.settings);
    assert_eq!(json["userName"], "Phúc Châu 🦀");
    assert_eq!(json["note"], "日本語テスト");
    let wrapped = read_json(&e.claw.join("wrapped.json"));
    assert_eq!(wrapped["command"], "echo héllo");
}

#[test]
fn extra_fields_survive_full_wrap_round_trip() {
    // install (wrap) -> uninstall (restore) must return the EXACT original object.
    // The uninstall half of this assertion lives in settings_uninstall_test.rs;
    // here we prove wrapped.json captures the full object.
    let e = env();
    let original_status_line = serde_json::json!({
        "command": "old.cmd", "padding": 2, "type": "command", "nested": {"deep": [1, 2, 3]}
    });
    let root = serde_json::json!({ "statusLine": original_status_line });
    std::fs::write(&e.settings, serde_json::to_string(&root).unwrap()).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    assert_eq!(read_json(&e.claw.join("wrapped.json")), original_status_line);
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p clawdometer-core --test settings_install_test`
Expected: all pass (Task 7's implementation strips BOM and preserves values through serde). If BOM/CRLF fails, fix `load_settings` only.

- [ ] **Step 3: Commit**

```bash
git add crates/core
git commit -m "test: install matrix — BOM/CRLF, unicode, wrap round-trip capture"
```

### Task 9: Core uninstall logic

**Files:**
- Modify: `crates/core/src/settings.rs`
- Test: `crates/core/tests/settings_uninstall_test.rs`

**Interfaces:**
- Consumes: `settings::{install, load/save internals}` from Task 7; `wrapped.json` format.
- Produces:
  - `settings::UninstallOutcome { Restored, RemovedKey, NotInstalled, NotOurs }` (derive `Debug, PartialEq`)
  - `settings::uninstall(settings_path: &Path, clawdometer_dir: &Path, our_command: &str) -> Result<UninstallOutcome, SettingsError>`

- [ ] **Step 1: Write failing tests** — `crates/core/tests/settings_uninstall_test.rs`

```rust
use std::path::{Path, PathBuf};

use clawdometer_core::settings::{install, uninstall, UninstallOutcome};

const OURS: &str = r#""C:\bin\clawdometer.exe" hook"#;

struct Env {
    _tmp: tempfile::TempDir,
    settings: PathBuf,
    claw: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    Env {
        settings: tmp.path().join("settings.json"),
        claw: tmp.path().join("clawdometer"),
        _tmp: tmp,
    }
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(
        std::fs::read_to_string(path).unwrap().trim_start_matches('\u{feff}'),
    ).unwrap()
}

#[test]
fn uninstall_restores_wrapped_original_exactly() {
    let e = env();
    let original = r#"{"model":"opus","statusLine":{"command":"old.cmd","padding":2,"nested":{"a":[1]}}}"#;
    std::fs::write(&e.settings, original).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();

    let outcome = uninstall(&e.settings, &e.claw, OURS).unwrap();
    assert_eq!(outcome, UninstallOutcome::Restored);

    let before: serde_json::Value = serde_json::from_str(original).unwrap();
    assert_eq!(read_json(&e.settings), before, "settings must round-trip deep-equal");
    assert!(!e.claw.join("wrapped.json").exists(), "wrapped.json consumed on restore");
}

#[test]
fn uninstall_removes_key_when_nothing_was_wrapped() {
    let e = env();
    std::fs::write(&e.settings, r#"{"model":"opus"}"#).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    let outcome = uninstall(&e.settings, &e.claw, OURS).unwrap();
    assert_eq!(outcome, UninstallOutcome::RemovedKey);
    let json = read_json(&e.settings);
    assert!(json.get("statusLine").is_none());
    assert_eq!(json["model"], "opus");
}

#[test]
fn uninstall_without_install_is_not_installed() {
    let e = env();
    std::fs::write(&e.settings, r#"{"model":"opus"}"#).unwrap();
    let outcome = uninstall(&e.settings, &e.claw, OURS).unwrap();
    assert_eq!(outcome, UninstallOutcome::NotInstalled);
    assert_eq!(read_json(&e.settings)["model"], "opus");
}

#[test]
fn uninstall_with_missing_settings_is_not_installed_and_creates_nothing() {
    let e = env();
    let outcome = uninstall(&e.settings, &e.claw, OURS).unwrap();
    assert_eq!(outcome, UninstallOutcome::NotInstalled);
    assert!(!e.settings.exists(), "uninstall must never create settings.json");
}

#[test]
fn uninstall_after_user_edit_warns_and_touches_nothing() {
    let e = env();
    std::fs::write(&e.settings, r#"{"statusLine":{"command":"old.cmd"}}"#).unwrap();
    install(&e.settings, &e.claw, OURS, "20260712-000000").unwrap();
    // user manually edits statusLine after install
    std::fs::write(&e.settings, r#"{"statusLine":{"command":"user-new-thing.cmd"}}"#).unwrap();
    let outcome = uninstall(&e.settings, &e.claw, OURS).unwrap();
    assert_eq!(outcome, UninstallOutcome::NotOurs);
    assert_eq!(read_json(&e.settings)["statusLine"]["command"], "user-new-thing.cmd");
    assert!(e.claw.join("wrapped.json").exists(), "wrapped.json left for manual recovery");
}

#[test]
fn uninstall_aborts_on_malformed_settings() {
    let e = env();
    std::fs::write(&e.settings, "{ nope").unwrap();
    assert!(uninstall(&e.settings, &e.claw, OURS).is_err());
    assert_eq!(std::fs::read_to_string(&e.settings).unwrap(), "{ nope");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p clawdometer-core --test settings_uninstall_test`
Expected: FAIL — `uninstall` not found.

- [ ] **Step 3: Implement `uninstall`** — append to `crates/core/src/settings.rs`

```rust
#[derive(Debug, PartialEq)]
pub enum UninstallOutcome {
    Restored,
    RemovedKey,
    NotInstalled,
    NotOurs,
}

pub fn uninstall(
    settings_path: &Path,
    clawdometer_dir: &Path,
    our_command: &str,
) -> Result<UninstallOutcome, SettingsError> {
    let (mut root, existed, _raw) = load_settings(settings_path)?;
    if !existed {
        return Ok(UninstallOutcome::NotInstalled);
    }
    let Some(current) = root.get(STATUSLINE_KEY) else {
        return Ok(UninstallOutcome::NotInstalled);
    };
    if !is_ours(current, our_command) {
        // User edited statusLine after install: warn, touch nothing.
        return Ok(UninstallOutcome::NotOurs);
    }
    let wrapped_path = clawdometer_dir.join("wrapped.json");
    if wrapped_path.exists() {
        let raw = std::fs::read_to_string(&wrapped_path)?;
        let original: Value = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
            .map_err(|e| SettingsError::MalformedSettings(format!("wrapped.json: {e}")))?;
        root[STATUSLINE_KEY] = original;
        save_settings(settings_path, &root)?;
        std::fs::remove_file(&wrapped_path)?;
        Ok(UninstallOutcome::Restored)
    } else {
        root.as_object_mut()
            .expect("load_settings guarantees object")
            .remove(STATUSLINE_KEY);
        save_settings(settings_path, &root)?;
        Ok(UninstallOutcome::RemovedKey)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clawdometer-core`
Expected: all core tests pass (install + uninstall + schema + state).

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat: settings.json uninstall — restore wrapped, remove key, refuse user edits"
```

### Task 10: CLI wiring — install / uninstall / status / --purge

**Files:**
- Create: `crates/cli/src/commands.rs`
- Modify: `crates/cli/src/main.rs`
- Test: `crates/cli/tests/install_cli_test.rs`

**Interfaces:**
- Consumes: `settings::{install, uninstall, InstallOutcome, UninstallOutcome}`, `paths::{clawdometer_dir, state_path, default_claude_settings_path}`, `state::{read_state, render_statusline}`.
- Produces:
  - `clawdometer install [--settings <path>]` — exit 0 on Installed/Wrapped/AlreadyInstalled; exit 1 + stderr message on malformed settings.
  - `clawdometer uninstall [--settings <path>] [--purge]` — exit 0 on Restored/RemovedKey/NotInstalled; exit 1 on NotOurs (warn) or malformed. Reports backup dir path. `--purge` deletes `clawdometer_dir()` after a successful uninstall.
  - `clawdometer status` — prints rendered statusline + `captured_at`, or "no state yet".
  - Our command string = `"<absolute exe path>" hook` (exe path quoted — paths contain spaces).

- [ ] **Step 1: Write failing tests** — `crates/cli/tests/install_cli_test.rs`

```rust
use std::path::Path;
use std::process::Command;

fn run(args: &[&str], claw_dir: &Path) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_clawdometer"))
        .args(args)
        .env("CLAWDOMETER_DIR", claw_dir)
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap(),
    )
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(
        std::fs::read_to_string(path).unwrap().trim_start_matches('\u{feff}'),
    ).unwrap()
}

#[test]
fn install_then_uninstall_round_trip_via_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("claw");
    std::fs::write(&settings, r#"{"model":"opus","statusLine":{"command":"old.cmd","padding":1}}"#).unwrap();

    let (stdout, _, code) = run(&["install", "--settings", settings.to_str().unwrap()], &claw);
    assert_eq!(code, 0, "install failed: {stdout}");
    let json = read_json(&settings);
    let cmd = json["statusLine"]["command"].as_str().unwrap();
    assert!(cmd.ends_with("\" hook"), "command is quoted exe + hook: {cmd}");
    assert!(cmd.contains("clawdometer"), "{cmd}");

    // second install: idempotent, still exit 0
    let (stdout, _, code) = run(&["install", "--settings", settings.to_str().unwrap()], &claw);
    assert_eq!(code, 0);
    assert!(stdout.to_lowercase().contains("already"), "{stdout}");

    let (_, _, code) = run(&["uninstall", "--settings", settings.to_str().unwrap()], &claw);
    assert_eq!(code, 0);
    let json = read_json(&settings);
    assert_eq!(json["statusLine"]["command"], "old.cmd");
    assert_eq!(json["statusLine"]["padding"], 1);
    assert_eq!(json["model"], "opus");
}

#[test]
fn uninstall_after_user_edit_exits_nonzero_and_touches_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("claw");
    std::fs::write(&settings, "{}").unwrap();
    run(&["install", "--settings", settings.to_str().unwrap()], &claw);
    std::fs::write(&settings, r#"{"statusLine":{"command":"user-edited.cmd"}}"#).unwrap();

    let (_, stderr, code) = run(&["uninstall", "--settings", settings.to_str().unwrap()], &claw);
    assert_ne!(code, 0, "user-edited statusLine must exit non-zero");
    assert!(!stderr.is_empty(), "must warn on stderr");
    assert_eq!(read_json(&settings)["statusLine"]["command"], "user-edited.cmd");
}

#[test]
fn install_malformed_settings_exits_nonzero() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("claw");
    std::fs::write(&settings, "{ nope").unwrap();
    let (_, stderr, code) = run(&["install", "--settings", settings.to_str().unwrap()], &claw);
    assert_ne!(code, 0);
    assert!(stderr.to_lowercase().contains("json"), "clear message required: {stderr}");
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ nope");
}

#[test]
fn purge_removes_clawdometer_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("claw");
    std::fs::write(&settings, "{}").unwrap();
    run(&["install", "--settings", settings.to_str().unwrap()], &claw);
    assert!(claw.exists() || !claw.exists()); // dir may or may not exist yet (no backup for fresh key)
    std::fs::create_dir_all(&claw).unwrap();
    let (_, _, code) = run(&["uninstall", "--settings", settings.to_str().unwrap(), "--purge"], &claw);
    assert_eq!(code, 0);
    assert!(!claw.exists(), "--purge removes the clawdometer dir");
}

#[test]
fn status_reports_no_state_then_state() {
    let tmp = tempfile::tempdir().unwrap();
    let claw = tmp.path().join("claw");
    let (stdout, _, code) = run(&["status"], &claw);
    assert_eq!(code, 0);
    assert!(stdout.to_lowercase().contains("no state"), "{stdout}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p clawdometer-cli --test install_cli_test`
Expected: FAIL — subcommands print "not implemented".

- [ ] **Step 3: Implement `crates/cli/src/commands.rs`**

```rust
use std::path::PathBuf;

use clawdometer_core::paths;
use clawdometer_core::settings::{
    install, uninstall, InstallOutcome, SettingsError, UninstallOutcome,
};
use clawdometer_core::state::{read_state, render_statusline};

/// `"<absolute exe path>" hook` — quoted because install paths contain spaces.
fn our_command() -> String {
    let exe = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("clawdometer.exe"));
    format!("\"{}\" hook", exe.display())
}

fn settings_path(args: &[String]) -> PathBuf {
    args.iter()
        .position(|a| a == "--settings")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(paths::default_claude_settings_path)
}

fn backup_timestamp() -> String {
    let fmt = time::format_description::parse("[year][month][day]-[hour][minute][second]")
        .expect("static format");
    time::OffsetDateTime::now_utc()
        .format(&fmt)
        .unwrap_or_else(|_| "unknown".into())
}

pub fn cmd_install(args: &[String]) -> i32 {
    let sp = settings_path(args);
    let claw = paths::clawdometer_dir();
    match install(&sp, &claw, &our_command(), &backup_timestamp()) {
        Ok(InstallOutcome::Installed) => {
            println!("installed: statusLine set in {}", sp.display());
            0
        }
        Ok(InstallOutcome::Wrapped) => {
            println!(
                "installed: previous statusLine preserved in {} and will be chained",
                claw.join("wrapped.json").display()
            );
            0
        }
        Ok(InstallOutcome::AlreadyInstalled) => {
            println!("already installed — nothing to do");
            0
        }
        Err(e) => {
            eprintln!("install aborted: {e}");
            1
        }
    }
}

pub fn cmd_uninstall(args: &[String]) -> i32 {
    let sp = settings_path(args);
    let claw = paths::clawdometer_dir();
    let purge = args.iter().any(|a| a == "--purge");
    let code = match uninstall(&sp, &claw, &our_command()) {
        Ok(UninstallOutcome::Restored) => {
            println!("uninstalled: original statusLine restored");
            0
        }
        Ok(UninstallOutcome::RemovedKey) => {
            println!("uninstalled: statusLine key removed");
            0
        }
        Ok(UninstallOutcome::NotInstalled) => {
            println!("not installed — nothing to do");
            0
        }
        Ok(UninstallOutcome::NotOurs) => {
            eprintln!(
                "statusLine was changed after install — refusing to touch it.\n\
                 Your original statusLine (if any) is in {}",
                claw.join("wrapped.json").display()
            );
            1
        }
        Err(e) => {
            eprintln!("uninstall aborted: {e}");
            1
        }
    };
    if code == 0 {
        if purge {
            let _ = std::fs::remove_dir_all(&claw);
            println!("purged {}", claw.display());
        } else if claw.exists() {
            println!("left on disk (remove with --purge): {}", claw.display());
        }
    }
    code
}

pub fn cmd_status() -> i32 {
    match read_state(&paths::state_path()) {
        Some(state) => {
            println!("{}", render_statusline(&state));
            println!("captured_at: {}", state.captured_at);
            0
        }
        None => {
            println!("no state yet — run a Claude Code session after `clawdometer install`");
            0
        }
    }
}
```

- [ ] **Step 4: Wire into `crates/cli/src/main.rs`**

```rust
mod commands;
mod hook;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("hook") => {
            let line = std::panic::catch_unwind(hook::run_hook)
                .unwrap_or_else(|_| String::from("clawdometer"));
            println!("{line}");
            0
        }
        Some("install") => commands::cmd_install(&args),
        Some("uninstall") => commands::cmd_uninstall(&args),
        Some("status") => commands::cmd_status(),
        _ => {
            eprintln!("usage: clawdometer <hook|install|uninstall|status> [--settings <path>] [--purge]");
            2
        }
    };
    std::process::exit(code);
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --workspace`
Expected: all pass.

Run: `cargo deny check bans`
Expected: `bans ok`.

- [ ] **Step 6: Commit**

```bash
git add crates/cli
git commit -m "feat: wire install/uninstall/status CLI with --settings and --purge"
```

---

## Phase 4 — Tauri HUD

**Milestone success criteria (verify all before Phase 5):**
- `cargo tauri dev` (or `cargo run -p clawdometer-app`) shows a frameless, always-on-top ~260×120 card.
- Editing `%USERPROFILE%\.clawdometer\state.json` by hand updates the HUD within ~2s (both via a fixture-derived state and by deleting the file → "waiting" state).
- Tray icon: tooltip shows `5h X% · 7d Y%`, left-click toggles the HUD, right-click menu has Show/Hide, Start with Windows, Quit.
- Dragging the card and restarting the app restores its position (`ui.json`).
- `resets_at` in the past renders "refresh pending"; `rate_limits: null` renders "waiting for Claude Code activity". No error dialog appears in any state.
- `cargo deny check bans` still passes (with any documented tauri-only wrapper exceptions).

### Task 11: Tauri v2 scaffold — window, tray, static UI shell

**Files:**
- Create: `app/src-tauri/Cargo.toml`, `app/src-tauri/tauri.conf.json`, `app/src-tauri/build.rs`, `app/src-tauri/src/main.rs`, `app/src-tauri/icons/` (generated), `app/src-tauri/capabilities/default.json`
- Create: `app/ui/index.html`, `app/ui/style.css`, `app/ui/main.js`
- Modify: `Cargo.toml` (workspace members), `deny.toml` (documented wrapper exceptions if needed)

**Interfaces:**
- Consumes: `clawdometer-core` (paths, state) — added as a path dependency.
- Produces: running Tauri app named `clawdometer-app`; window label `hud`; tray id `main`. Event channel name `state-updated` (payload = `state.json` contents as JSON) that Task 12 emits and Task 13's JS consumes.

- [ ] **Step 1: Install prerequisites**

```powershell
cargo install tauri-cli --version "^2" --locked
```

Expected: `cargo tauri --version` prints a 2.x version. (WebView2 ships with Windows 11 — no action.)

- [ ] **Step 2: Add workspace member**

Workspace root `Cargo.toml`:

```toml
members = ["crates/core", "crates/cli", "app/src-tauri"]
```

- [ ] **Step 3: Create `app/src-tauri/Cargo.toml`**

```toml
[package]
name = "clawdometer-app"
version = "0.1.0"
edition = "2021"
license = "MIT"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-autostart = "2"
clawdometer-core = { path = "../../crates/core" }
serde_json = "1"
notify = "6"
```

`app/src-tauri/build.rs`:

```rust
fn main() {
    tauri_build::build()
}
```

- [ ] **Step 4: Create `app/src-tauri/tauri.conf.json`**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Clawdometer",
  "version": "0.1.0",
  "identifier": "io.github.clawdometer",
  "build": {
    "frontendDist": "../ui"
  },
  "app": {
    "withGlobalTauri": true,
    "windows": [
      {
        "label": "hud",
        "title": "Clawdometer",
        "width": 260,
        "height": 120,
        "decorations": false,
        "alwaysOnTop": true,
        "skipTaskbar": true,
        "resizable": false,
        "visible": true,
        "shadow": false
      }
    ]
  },
  "bundle": {
    "active": true,
    "targets": ["nsis"],
    "icon": ["icons/icon.ico"]
  }
}
```

Generate placeholder icons: `cargo tauri icon` with any 1024×1024 PNG (create a plain colored square with PowerShell/mspaint if nothing else is at hand); commit the generated `app/src-tauri/icons/`.

`app/src-tauri/capabilities/default.json` (minimal — event listening only, no fs/network capabilities exposed to the webview):

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "windows": ["hud"],
  "permissions": ["core:event:default", "core:window:default", "core:window:allow-start-dragging"]
}
```

- [ ] **Step 5: Create `app/src-tauri/src/main.rs` (shell only — watcher comes in Task 12)**

```rust
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

fn toggle_hud(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("hud") {
        let visible = win.is_visible().unwrap_or(false);
        if visible {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let show_hide = MenuItem::with_id(app, "toggle", "Show/Hide", true, None::<&str>)?;
            let autostart = MenuItem::with_id(app, "autostart", "Start with Windows", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_hide, &autostart, &quit])?;

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().expect("bundled icon").clone())
                .tooltip("Clawdometer")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, .. } = event {
                        toggle_hud(tray.app_handle());
                    }
                })
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "toggle" => toggle_hud(app),
                    "autostart" => {
                        // Explicit user action — the one write outside ~/.clawdometer
                        // (HKCU Run key), documented in the README.
                        use tauri_plugin_autostart::ManagerExt;
                        let mgr = app.autolaunch();
                        let enabled = mgr.is_enabled().unwrap_or(false);
                        let _ = if enabled { mgr.disable() } else { mgr.enable() };
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run clawdometer app");
}
```

- [ ] **Step 6: Create the static UI shell**

`app/ui/index.html`:

```html
<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <link rel="stylesheet" href="style.css" />
</head>
<body data-tauri-drag-region>
  <div id="card" data-tauri-drag-region>
    <div id="model" data-tauri-drag-region>Clawdometer</div>
    <div class="row">
      <span class="label">5h</span>
      <div class="bar"><div id="bar5h" class="fill"></div></div>
      <span id="txt5h" class="pct">—</span>
    </div>
    <div class="row">
      <span class="label">7d</span>
      <div class="bar"><div id="bar7d" class="fill"></div></div>
      <span id="txt7d" class="pct">—</span>
    </div>
    <div id="footer">waiting for Claude Code activity</div>
  </div>
  <script src="main.js"></script>
</body>
</html>
```

`app/ui/style.css`:

```css
* { margin: 0; padding: 0; box-sizing: border-box; user-select: none; }
html, body { background: transparent; overflow: hidden; }
#card {
  width: 260px; height: 120px; padding: 10px 12px;
  background: rgba(24, 24, 28, 0.92); color: #e8e8ea;
  border-radius: 10px; font: 12px/1.5 "Segoe UI", sans-serif;
  display: flex; flex-direction: column; gap: 4px;
}
#model { font-weight: 600; font-size: 12px; }
.row { display: flex; align-items: center; gap: 6px; }
.label { width: 18px; color: #9a9aa2; }
.bar { flex: 1; height: 8px; background: #35353c; border-radius: 4px; overflow: hidden; }
.fill { height: 100%; width: 0%; background: #d97706; border-radius: 4px; transition: width .3s; }
.pct { min-width: 92px; text-align: right; color: #c9c9cf; font-variant-numeric: tabular-nums; }
#footer { margin-top: auto; color: #7a7a82; font-size: 11px; }
```

`app/ui/main.js` — placeholder for Task 13:

```js
// State rendering wired in Task 13.
```

- [ ] **Step 7: Verify it runs + cargo-deny**

Run: `cargo tauri dev` from `app/src-tauri` (or `cargo run -p clawdometer-app` after a first `cargo tauri dev` generated schemas).
Expected: frameless dark card appears, always on top, draggable; tray icon present; left-click toggles; Quit works.

Run: `cargo deny check bans`
Expected: `bans ok`. If a Tauri transitive dep trips a ban, add `wrappers = ["<direct tauri crate>"]` to that single entry in `deny.toml` with a comment naming the transitive path — never for core/cli deps.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml deny.toml app
git commit -m "feat: Tauri v2 HUD shell — frameless always-on-top card with tray"
```

### Task 12: state.json watcher → `state-updated` event

**Files:**
- Create: `app/src-tauri/src/watcher.rs`
- Modify: `app/src-tauri/src/main.rs`
- Test: `app/src-tauri/src/watcher.rs` (unit tests in-module)

**Interfaces:**
- Consumes: `clawdometer_core::{paths::state_path, state::read_state}`.
- Produces: Tauri event `state-updated` emitted app-wide. Payload JSON: `{"state": <State as JSON> | null, "received_at_ms": <i64 unix millis>}`. Emitted on startup, on every observed file change, and from a 2s fallback poll when content changed. Skin contract per spec: this single event is the entire core↔skin interface.

- [ ] **Step 1: Write failing unit test** — in `app/src-tauri/src/watcher.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_null_state_when_file_missing_or_torn() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("state.json");
        assert!(build_payload(&missing)["state"].is_null());
        std::fs::write(&missing, "{ torn").unwrap();
        assert!(build_payload(&missing)["state"].is_null());
    }

    #[test]
    fn payload_carries_state_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let raw = include_str!("../../../crates/core/tests/fixtures/stdin-with-limits.json");
        let input = clawdometer_core::schema::parse_statusline_input(raw).unwrap();
        let state = clawdometer_core::state::State::from_input(&input, "t".into());
        clawdometer_core::state::write_state_atomic(&path, &state).unwrap();
        let payload = build_payload(&path);
        assert_eq!(payload["state"]["rate_limits"]["five_hour"]["used_percentage"], 1);
        assert_eq!(payload["state"]["captured_at"], "t");
    }
}
```

Add `tempfile = "3"` to `app/src-tauri/Cargo.toml` `[dev-dependencies]`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p clawdometer-app`
Expected: FAIL — `build_payload` not found.

- [ ] **Step 3: Implement `app/src-tauri/src/watcher.rs`**

```rust
use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

pub const STATE_EVENT: &str = "state-updated";

pub fn build_payload(state_path: &Path) -> serde_json::Value {
    let state = clawdometer_core::state::read_state(state_path)
        .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null))
        .unwrap_or(serde_json::Value::Null);
    let received_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    serde_json::json!({ "state": state, "received_at_ms": received_at_ms })
}

/// notify watcher on ~/.clawdometer + 2s fallback poll. Emits only when the
/// serialized payload's state differs from the last emission (poll path) or
/// the FS reports a change (watch path). Never panics the app: watch setup
/// failure degrades to poll-only.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let state_path: PathBuf = clawdometer_core::paths::state_path();
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

        let mut last_state = serde_json::Value::Null;
        // initial emission so the UI renders immediately
        let payload = build_payload(&state_path);
        last_state = payload["state"].clone();
        let _ = app.emit(STATE_EVENT, &payload);

        loop {
            // wake on FS event or every 2s (debounce fallback poll)
            let _ = rx.recv_timeout(Duration::from_secs(2));
            let payload = build_payload(&state_path);
            if payload["state"] != last_state {
                last_state = payload["state"].clone();
                let _ = app.emit(STATE_EVENT, &payload);
            }
        }
    });
}
```

- [ ] **Step 4: Wire into `main.rs` setup**

Add `mod watcher;` at the top and inside `.setup(|app| { ... })`, before `Ok(())`:

```rust
            watcher::spawn(app.handle().clone());
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p clawdometer-app`
Expected: 2 passed.

- [ ] **Step 6: Commit**

```bash
git add app
git commit -m "feat: watch state.json and emit state-updated event to the skin"
```

### Task 13: Default skin — bars, reset times, data age, waiting states

**Files:**
- Modify: `app/ui/main.js`

**Interfaces:**
- Consumes: `state-updated` event payload from Task 12: `{state: {captured_at, rate_limits: {five_hour: {used_percentage, resets_at}, seven_day: {...}} | null, model: {display_name} | null, ...} | null, received_at_ms}`.
- Produces: complete default skin behavior. All display rules from the spec HUD section.

- [ ] **Step 1: Implement `app/ui/main.js`**

```js
// Default skin. Contract: the single `state-updated` event. No other IPC.
const els = {
  model: document.getElementById("model"),
  bar5h: document.getElementById("bar5h"),
  bar7d: document.getElementById("bar7d"),
  txt5h: document.getElementById("txt5h"),
  txt7d: document.getElementById("txt7d"),
  footer: document.getElementById("footer"),
};

let current = null; // last payload

function fmtReset(resetsAtEpochSec, nowMs) {
  if (resetsAtEpochSec * 1000 < nowMs) return "refresh pending";
  const d = new Date(resetsAtEpochSec * 1000);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `resets ${hh}:${mm}`;
}

function renderWindow(win, barEl, txtEl, nowMs) {
  if (!win || typeof win.used_percentage !== "number") {
    barEl.style.width = "0%";
    txtEl.textContent = "—";
    return;
  }
  const pct = Math.max(0, Math.min(100, win.used_percentage));
  barEl.style.width = pct + "%";
  barEl.style.background = pct >= 90 ? "#dc2626" : pct >= 70 ? "#f59e0b" : "#d97706";
  txtEl.textContent = `${win.used_percentage}% · ${fmtReset(win.resets_at, nowMs)}`;
}

function fmtAge(capturedAtIso, nowMs) {
  const t = Date.parse(capturedAtIso);
  if (Number.isNaN(t)) return "";
  const mins = Math.floor((nowMs - t) / 60000);
  if (mins < 1) return "as of just now";
  if (mins < 60) return `as of ${mins}m ago`;
  const hours = Math.floor(mins / 60);
  return `as of ${hours}h ${mins % 60}m ago`;
}

function render() {
  const nowMs = Date.now();
  const state = current && current.state;
  if (!state || !state.rate_limits) {
    els.model.textContent = (state && state.model && state.model.display_name) || "Clawdometer";
    renderWindow(null, els.bar5h, els.txt5h, nowMs);
    renderWindow(null, els.bar7d, els.txt7d, nowMs);
    els.footer.textContent = "waiting for Claude Code activity";
    return;
  }
  els.model.textContent = (state.model && state.model.display_name) || "Claude";
  renderWindow(state.rate_limits.five_hour, els.bar5h, els.txt5h, nowMs);
  renderWindow(state.rate_limits.seven_day, els.bar7d, els.txt7d, nowMs);
  els.footer.textContent = fmtAge(state.captured_at, nowMs);
}

window.__TAURI__.event.listen("state-updated", (event) => {
  current = event.payload;
  render();
});

// Age line ticks locally between updates.
setInterval(render, 30000);
render();
```

- [ ] **Step 2: Manual verification with real fixture data**

With the app running (`cargo tauri dev`) and `CLAWDOMETER_DIR` NOT set (real `~/.clawdometer`):

```powershell
Get-Content crates/core/tests/fixtures/stdin-with-limits.json | .\target\debug\clawdometer.exe hook
```

Expected within ~2s: model name "Opus 4.8 (1M context)", 5h bar at 1% with `1% · refresh pending` (the fixture's `resets_at` epochs are in the past relative to today — this exercises the "refresh pending" rule), 7d bar at 5%, footer "as of just now".

Then: `Remove-Item $env:USERPROFILE\.clawdometer\state.json`
Expected within ~2s: bars empty, footer "waiting for Claude Code activity". No dialog, no crash.

Then write a hand-edited state.json with a future `resets_at` (e.g. current epoch + 3600) and confirm `resets HH:MM` renders a sane local time.

- [ ] **Step 3: Commit**

```bash
git add app/ui
git commit -m "feat: default skin — limit bars, reset times, data age, waiting states"
```

### Task 14: Position persistence + tray tooltip percentages

**Files:**
- Modify: `app/src-tauri/src/main.rs`
- Modify: `app/src-tauri/src/watcher.rs`
- Test: unit test for ui.json round-trip in `main.rs`'s module (or a small `ui_prefs.rs`)

**Interfaces:**
- Consumes: window `hud`, tray `main`, `watcher::build_payload`.
- Produces: `~/.clawdometer/ui.json` = `{"x": <i32>, "y": <i32>}`; tray tooltip `5h X% · 7d Y%` refreshed on every state emission.

- [ ] **Step 1: Write failing test** — create `app/src-tauri/src/ui_prefs.rs`

```rust
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UiPrefs {
    pub x: i32,
    pub y: i32,
}

pub fn load(path: &Path) -> Option<UiPrefs> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub fn save(path: &Path, prefs: UiPrefs) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(body) = serde_json::to_string(&prefs) {
        let _ = std::fs::write(path, body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.json");
        assert!(load(&path).is_none());
        save(&path, UiPrefs { x: -100, y: 2000 });
        assert_eq!(load(&path), Some(UiPrefs { x: -100, y: 2000 }));
        std::fs::write(&path, "garbage").unwrap();
        assert!(load(&path).is_none());
    }
}
```

Run: `cargo test -p clawdometer-app` — write the test first with only the types stubbed if you want a strict red step; the module above is small enough that test+impl land together. Expected: PASS.

- [ ] **Step 2: Wire position persistence in `main.rs`**

Add `mod ui_prefs;`. In `.setup`, after the tray build:

```rust
            // Restore HUD position
            let ui_path = clawdometer_core::paths::clawdometer_dir().join("ui.json");
            if let (Some(win), Some(prefs)) =
                (app.get_webview_window("hud"), ui_prefs::load(&ui_path))
            {
                let _ = win.set_position(tauri::PhysicalPosition::new(prefs.x, prefs.y));
            }
```

Add a window-event handler on the builder (before `.run`):

```rust
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Moved(pos) = event {
                if window.label() == "hud" {
                    let ui_path = clawdometer_core::paths::clawdometer_dir().join("ui.json");
                    ui_prefs::save(&ui_path, ui_prefs::UiPrefs { x: pos.x, y: pos.y });
                }
            }
        })
```

- [ ] **Step 3: Tray tooltip from state** — in `watcher.rs`, after each `app.emit(...)` call add:

```rust
            update_tooltip(&app, &payload);
```

and the helper:

```rust
fn update_tooltip(app: &AppHandle, payload: &serde_json::Value) {
    let text = match (
        payload.pointer("/state/rate_limits/five_hour/used_percentage").and_then(|v| v.as_i64()),
        payload.pointer("/state/rate_limits/seven_day/used_percentage").and_then(|v| v.as_i64()),
    ) {
        (Some(fh), Some(sd)) => format!("5h {fh}% · 7d {sd}%"),
        _ => String::from("Clawdometer — waiting for data"),
    };
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(text));
    }
}
```

(Adjust both emit sites — initial emission and loop emission.)

- [ ] **Step 4: Manual verification**

Run app, drag the card somewhere distinctive, quit via tray, relaunch.
Expected: card reappears at the dragged position; `%USERPROFILE%\.clawdometer\ui.json` contains the coordinates. Fire the hook with the fixture; hover tray icon → tooltip `5h 1% · 7d 5%`.

- [ ] **Step 5: Commit**

```bash
git add app
git commit -m "feat: HUD position persistence and live tray tooltip"
```

---

## Phase 5 — Integration + manual acceptance

**Milestone success criteria (v1 done when all check out):**
- Full workspace: `cargo test --workspace` green, `cargo deny check bans` green, `cargo build --release` produces `clawdometer.exe` and the Tauri app.
- End-to-end script test passes: install into a sandbox settings.json → pipe both fixtures through the INSTALLED command string → state.json correct → uninstall restores deep-equal.
- Manual acceptance on the dev machine (real `~/.claude/settings.json`) signed off per the checklist below.
- README safety statements present.

### Task 15: End-to-end integration test

**Files:**
- Test: `crates/cli/tests/e2e_test.rs`

**Interfaces:**
- Consumes: everything from Phases 1–3 via the built binary only (no library calls except settings verification).

- [ ] **Step 1: Write the E2E test** — `crates/cli/tests/e2e_test.rs`

```rust
//! Full lifecycle against a sandboxed settings.json:
//! install -> run the EXACT installed command string with fixture stdin ->
//! verify state.json -> uninstall -> deep-equal restore.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const PRE: &str = include_str!("../../core/tests/fixtures/stdin-pre-response.json");
const FULL: &str = include_str!("../../core/tests/fixtures/stdin-with-limits.json");

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(
        std::fs::read_to_string(path).unwrap().trim_start_matches('\u{feff}'),
    ).unwrap()
}

#[test]
fn full_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("settings.json");
    let claw = tmp.path().join("claw");
    let original = r#"{"model":"opus","statusLine":{"command":"echo pre-existing","padding":1}}"#;
    std::fs::write(&settings, original).unwrap();

    // 1. install
    let status = Command::new(env!("CARGO_BIN_EXE_clawdometer"))
        .args(["install", "--settings", settings.to_str().unwrap()])
        .env("CLAWDOMETER_DIR", &claw)
        .status()
        .unwrap();
    assert!(status.success());

    // 2. run the EXACT command Claude Code would run (from settings.json, via cmd /C)
    let installed_cmd = read_json(&settings)["statusLine"]["command"]
        .as_str().unwrap().to_string();
    for (fixture, expect_limits) in [(PRE, false), (FULL, true)] {
        let mut child = shell(&installed_cmd)
            .env("CLAWDOMETER_DIR", &claw)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(fixture.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "installed command must exit 0");
        // wrapped pre-existing statusline is chained through
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "pre-existing");
        let state = clawdometer_core::state::read_state(&claw.join("state.json")).unwrap();
        assert_eq!(state.rate_limits.is_some(), expect_limits);
    }

    // 3. uninstall restores original deep-equal
    let status = Command::new(env!("CARGO_BIN_EXE_clawdometer"))
        .args(["uninstall", "--settings", settings.to_str().unwrap()])
        .env("CLAWDOMETER_DIR", &claw)
        .status()
        .unwrap();
    assert!(status.success());
    let before: serde_json::Value = serde_json::from_str(original).unwrap();
    assert_eq!(read_json(&settings), before);
}

#[cfg(windows)]
fn shell(command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").raw_arg(command);
    cmd
}

#[cfg(not(windows))]
fn shell(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p clawdometer-cli --test e2e_test`
Expected: PASS. If the installed-command invocation fails on quoting, fix `our_command()` quoting in `commands.rs` (Task 10) — the test is the arbiter.

- [ ] **Step 3: Full-suite gate**

Run: `cargo test --workspace && cargo deny check bans && cargo build --release`
Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add crates/cli
git commit -m "test: end-to-end lifecycle — install, real-fixture hook via installed command, uninstall"
```

### Task 16: README safety statements + manual acceptance

**Files:**
- Create: `README.md`
- Create: `docs/acceptance-v1.md` (filled-in checklist, committed as the acceptance record)

**Interfaces:**
- Consumes: the complete built system.

- [ ] **Step 1: Write `README.md`**

Must contain, prominently near the top (spec safety invariant 6):

```markdown
# Clawdometer

Unofficial Windows desktop HUD for Claude Code usage limits.

> **Unofficial.** Not affiliated with or endorsed by Anthropic.
> **Local-files-only.** Clawdometer reads only local files written by Claude
> Code on your machine and makes **zero network requests** (enforced in CI via
> a cargo-deny ban on network crates). It never reads credentials.
> **Writes:** only `~/.clawdometer/` and the `statusLine` key of
> `~/.claude/settings.json` (during `install`/`uninstall`). Exception: the
> tray's "Start with Windows" toggle writes the standard HKCU Run registry
> key, only when you click it.
```

Plus: install/uninstall usage (`clawdometer install`, `clawdometer uninstall [--purge]`), what the HUD shows, note that percentages have 1% granularity (same as `/usage`), that data appears only after Claude Code fires the statusline (some surfaces don't), MIT license.

- [ ] **Step 2: Run manual acceptance on the dev machine** — record each item in `docs/acceptance-v1.md` with pass/fail and the observed values:

```markdown
# v1 Manual Acceptance — 2026-MM-DD

- [ ] `Copy-Item ~/.claude/settings.json ~/settings-before-acceptance.json` (manual safety copy)
- [ ] `clawdometer install` against the REAL ~/.claude/settings.json exits 0 and reports the backup path
- [ ] Backup file exists under ~/.clawdometer/backups/ and byte-matches the pre-install settings.json
- [ ] Start a real interactive Claude Code terminal session; statusline renders (ours or chained original)
- [ ] After the first API response, HUD shows 5h/7d percentages
- [ ] HUD percentages match `/usage` inside the session (±1% granularity)
- [ ] Reset times in HUD are sane local times (cross-check `/usage`)
- [ ] Kill the session; HUD "as of Xm ago" footer ticks upward
- [ ] Tray tooltip shows `5h X% · 7d Y%`; left-click toggles HUD; drag + relaunch restores position
- [ ] `clawdometer uninstall` exits 0; settings.json deep-equals the safety copy
      (compare: parse both, assert equal — statusLine restored or removed as appropriate)
- [ ] Delete ~/settings-before-acceptance.json
```

- [ ] **Step 3: Commit**

```bash
git add README.md docs/acceptance-v1.md
git commit -m "docs: README safety statements and v1 acceptance record"
```

---

## Self-review record

- **Spec coverage:** CLI subcommands (Tasks 4–10), Tauri HUD incl. tray/drag/persistence/waiting/refresh-pending (11–14), installer full matrix (7–9 + CLI rows in 10), data flow incl. wrapped chaining + 2s timeout (6), atomic state.json + last-write-wins (3), error rules (5, 12, 13), safety invariants (deny.toml Task 1, read-only reads — no writes to `~/.claude` outside `install`/`uninstall`, README Task 16), testing strategy (fixtures Tasks 2–4, error-injection Task 5, E2E Task 15, manual acceptance Task 16). Out of scope confirmed out: tailer, history, skins marketplace.
- **Known deviations, deliberate:** (a) fixture-over-spec on pre-response `context_window` (documented in header); (b) "Start with Windows" registry write documented as the single exception to write-scope invariant 4.
- **Type consistency:** `InstallOutcome::{Installed,Wrapped,AlreadyInstalled}`, `UninstallOutcome::{Restored,RemovedKey,NotInstalled,NotOurs}`, `State::from_input`, `render_statusline`, `read_state`, `write_state_atomic`, `build_payload`, event name `state-updated` — used identically at every site.
