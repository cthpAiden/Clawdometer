# Clawdometer — Design Spec (v1)

**Date:** 2026-07-12
**Status:** Approved pending user review
**Verified against:** Claude Code CLI 2.1.205, Windows 11, real statusline stdin dumps + real transcript JSONL.

## What it is

Unofficial, open-source (MIT) Windows desktop HUD showing Claude Code usage limits in real time. Reads only local files. Makes zero network requests. Not affiliated with Anthropic.

## v1 scope

- `clawdometer.exe` CLI: `install`, `uninstall`, `hook`, `status`.
- Tauri v2 widget: tray icon + toggleable always-on-top mini HUD showing 5-hour and 7-day limit percentages, reset times, model, and data age.
- Out of scope for v1 (designed for, shipped later): JSONL token tailer (v1.1), history/stats view (v1.2), skin marketplace.

## Verified data schemas (empirical, not from docs)

### Statusline stdin (Source A — headline feature)

Claude Code invokes the configured `statusLine` command on every statusline refresh, passing JSON on stdin. Captured from CLI 2.1.205:

- **Pre-first-API-response:** NO `rate_limits` key, NO `context_window` key. Present: `session_id`, `transcript_path`, `cwd`, `model{id,display_name}`, `workspace`, `version`, `cost`, `output_style`, `exceeds_200k_tokens`, `fast_mode`, `thinking`.
- **After first response**, additionally:

```json
"rate_limits": {
  "five_hour": { "used_percentage": 1, "resets_at": 1783814400 },
  "seven_day": { "used_percentage": 5, "resets_at": 1784170800 }
},
"context_window": { "used_percentage": 4, "remaining_percentage": 96, "...": "..." }
```

- `used_percentage` is an **integer** (1% granularity — same as `/usage`).
- `resets_at` is unix epoch seconds (verified: decodes to sane local times).
- `rate_limits` absent for API-key users, pre-first-response, and older CLIs. Absence is a normal state, not an error.
- Statusline does NOT fire in all surfaces (desktop-app session did not fire it; interactive terminal session did). Data staleness is therefore a first-class UI concept.

### Transcript JSONL (Source B — v1.1, schema locked now)

`~/.claude/projects/<slug>/<session-uuid>.jsonl`, one JSON object per line. Assistant lines carry:

```json
"message": {
  "model": "claude-fable-5",
  "usage": {
    "input_tokens": 2,
    "cache_creation_input_tokens": 1071,
    "cache_read_input_tokens": 64852,
    "output_tokens": 425,
    "service_tier": "standard",
    "cache_creation": { "ephemeral_1h_input_tokens": 1071, "ephemeral_5m_input_tokens": 0 },
    "iterations": [ "..." ],
    "speed": "standard"
  }
},
"timestamp": "...", "sessionId": "...", "uuid": "..."
```

**Constraints for v1.1 (recorded now so the design doesn't paint us in):**
- Token sums are ACTIVITY indicators, never "% toward limit". Cache reads dominate raw sums (sample: 2 input vs 64,852 cache-read) and Anthropic's limit weighting is opaque. Only `rate_limits` is ground truth for limits. UI must never present token counts as limit math.
- Dedup required: multiple transcript lines can share a `message.id` (streaming/tool-use continuations); dedup by `message.id` + `requestId` before summing.
- `~/.claude/stats-cache.json` is stale (verified: recomputed ~weekly on this machine) — history view only, never live numbers.

## Architecture

```
clawdometer/            (Rust workspace)
├─ crates/
│  ├─ core/             # data layer: stdin schema, state.json IO (atomic), settings.json merge
│  └─ cli/              # clawdometer.exe: hook | install | uninstall | status
├─ app/                 # Tauri v2 host (thin)
│  └─ ui/               # webview assets = default skin
└─ docs/
```

`core` is a pure library with no UI dependencies — future TUI frontends and alternative skins consume it. Skin contract = the single `StateUpdated` JSON event pushed from Tauri host to webview; skins are asset swaps, zero core changes.

### Data flow

1. Claude Code fires statusline → `clawdometer.exe hook`:
   - reads stdin JSON,
   - atomically writes `~/.clawdometer/state.json` (temp file in same dir + rename),
   - if `~/.clawdometer/wrapped.json` exists: spawns the user's original statusline command with the same stdin, passes its stdout through (2s timeout → fall back to own output);
   - else prints own one-line statusline (model + 5h/7d %).
   - Entire hook is wrapped in a catch-all: any internal error still prints a statusline line and exits 0. The hook must NEVER break the user's statusline. Budget: <100ms typical (compiled Rust, ~5–15ms startup).
2. Tauri app watches `state.json` (file-watch + 2s debounce fallback poll) → pushes `StateUpdated` event to webview.
3. Concurrent sessions: all fire the hook; last-write-wins is correct (limits are account-wide); atomic rename prevents torn reads.

### state.json (v1, `schema_version: 1`)

```json
{
  "schema_version": 1,
  "captured_at": "2026-07-12T02:02:16Z",
  "rate_limits": { "five_hour": {"used_percentage": 1, "resets_at": 1783814400},
                    "seven_day": {"used_percentage": 5, "resets_at": 1784170800} },
  "model": { "id": "claude-opus-4-8[1m]", "display_name": "Opus 4.8 (1M context)" },
  "context_window": { "used_percentage": 4 },
  "session_id": "…", "transcript_path": "…", "cli_version": "2.1.205"
}
```

- `rate_limits` and `context_window` are `null` when absent from stdin (normal pre-response state).
- `captured_at` = hook wall-clock at write; powers the "as of Xm ago" display.
- `transcript_path` stored now as groundwork for the v1.1 tailer.
- Every field verified against real captured dumps (2026-07-12).

## Installer / uninstaller (riskiest code — hardest tested)

`clawdometer install`:
1. Read `~/.claude/settings.json`. Missing → treat as `{}`. Malformed JSON → abort with clear message, touch nothing.
2. Backup to `~/.clawdometer/backups/settings-<timestamp>.json` (never overwrite backups).
3. Cases:
   - No `statusLine` → set it to our command.
   - Existing statusLine (not ours) → persist the FULL original `statusLine` object (command + any extra fields like `padding`) to `~/.clawdometer/wrapped.json`, then set statusLine to ours. Hook chains it per data-flow above.
   - Already ours → no-op, "already installed".
4. Write via parse → modify only `statusLine` → serialize → temp file + atomic rename. All other keys must survive semantically intact (deep-equal test).

`clawdometer uninstall`:
- `wrapped.json` present → restore original statusLine object; else remove `statusLine` key.
- If current statusLine is not ours (user edited after install) → warn, touch nothing, exit non-zero.
- Leaves backups and `~/.clawdometer` on disk (reports paths); `--purge` removes own dir.
- Never touches any other settings.json key.

### Installer test matrix

missing settings.json · empty `{}` · existing statusLine · statusLine with extra fields (preserved through wrap/unwrap round-trip) · BOM / CRLF · unicode values · install twice (idempotent) · uninstall without install · uninstall after manual user edit · malformed JSON (abort untouched).

## HUD (default skin)

- Frameless, always-on-top, draggable card (~260×120), position persisted to `~/.clawdometer/ui.json`.
- Shows: two progress bars (5h / 7d) with `X% · resets HH:MM`, model display name, data age ("as of 2m ago", ticks locally between updates).
- No data yet / `rate_limits: null` → "waiting for Claude Code activity". Never an error dialog.
- `resets_at` in the past → "refresh pending", never a negative countdown.
- Tray: tooltip `5h X% · 7d Y%`, left-click toggles HUD, right-click menu (Show/Hide, Start with Windows, Quit).

## Error-handling rules

- Hook: fail silent, always emit statusline text, always exit 0.
- Widget: missing/malformed/torn state.json → "waiting" state, retry next cycle. No crashes, no dialogs.
- Wrapped user command failure/timeout → fall back to own statusline output.

## Safety invariants (product identity — enforced, not aspirational)

1. NO network calls anywhere in the workspace. Enforced via `cargo-deny` ban on network crates + CI check.
2. NO reads of `~/.claude/.credentials.json` or any credential store.
3. NO impersonation of the Claude Code CLI.
4. Writes ONLY under `~/.clawdometer/` + the `statusLine` key of settings.json during install/uninstall.
5. All `~/.claude` reads opened read-only with shared-read flags.
6. README states prominently: unofficial, not affiliated with Anthropic, local-files-only, zero API requests.

## Testing strategy

- **core unit tests:** settings merge matrix (above), state.json round-trip, stdin parse fixtures = the two real captured dumps (with and without `rate_limits`).
- **CLI integration tests:** pipe real dump files into `clawdometer.exe hook`, assert state.json content + stdout + exit 0; error-injection (unwritable dir, garbage stdin) still exits 0 with output.
- **Manual acceptance (final milestone):** install on the dev machine, run a real session, HUD percentages match `/usage`, uninstall restores settings.json exactly.

## Decisions log

| Decision | Choice | Why |
|---|---|---|
| Widget shell | Tauri v2 | WebView2 ships with Win11; HTML/CSS skins cleanly separated from Rust core; tray + overlay both supported; cross-platform later |
| Hook language | Same Rust binary, `hook` subcommand | one artifact, ~5–15ms startup, shared tested code. PowerShell rejected (200–500ms cold start breaks <100ms budget) |
| License | MIT | max adoption; paid skins are separate non-OSS assets, unconstrained by core license |
| v1 scope | Limits HUD + installer only | installer is riskiest, gets full test attention; tailer edge cases deferred to v1.1 |
| Form factor | Tray + toggleable overlay | passive glance value + stays out of the way |
