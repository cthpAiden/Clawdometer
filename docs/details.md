# Clawdometer: full details

The long version. For the quick start, see the [README](../README.md).

## What it does

Claude Code sends usage data (rate-limit percentages, reset times, model,
context window) to your statusline command on every API response. Clawdometer
installs itself as that statusline command (automatically on first HUD launch
if you have no statusline configured), records the latest snapshot to
`~/.clawdometer/state.json`, and shows the 5-hour and 7-day percentages in a
small always-on-top HUD and a system-tray tooltip (`5h X% · 7d Y%`).

When the snapshot goes stale (you're using claude.ai web/mobile or the
desktop app instead of a statusline-fed terminal session), the HUD refreshes
by running the **official CLI headlessly** — `claude -p /usage` — and
parsing the report Claude Code itself prints (about once a minute while no
fresher data is coming in, or on demand via tray → *Refresh usage*).

Those are the only two data sources, and both are official Claude Code
surfaces — Clawdometer makes **zero network requests of its own** and never
touches your credentials. (Earlier versions polled Anthropic's usage
endpoint with Claude Code's OAuth token; that was removed: Anthropic's
Consumer Terms of Service prohibit using OAuth tokens from Claude accounts
in third-party tools, and this app should never put your account at risk.)

The HUD header shows a countdown to the 5-hour window reset (limits are
account-wide, so a model name would add nothing). When a window's reset time
passes while your machine is idle, the HUD derives the truthful 0% locally —
no network needed. (Usage made on claude.ai web/mobile only shows up on the
next Claude Code response.)

If you already had a statusline configured, the HUD leaves it alone —
run `clawdometer install` (CLI) to chain it: your original statusline still
renders its output (with a 2-second timeout), and `uninstall` restores it
exactly.

## Security

Clawdometer never asks for, reads, or stores any credential, and makes **no
network requests of any kind** — every binary (hook, CLI, HUD) is compiled
under a `cargo-deny` ban on all HTTP/TLS crates, so this is provable from the
build, not a promise. There is **no telemetry**. Usage data arrives only via
Claude Code's own surfaces: its statusline hook (Claude Code runs the hook
and pipes it a JSON snapshot), and a periodic headless `claude -p /usage`
run whose plain-text output the HUD parses — in both cases Claude Code
itself fetches the numbers with its own sign-in, and the only process
Clawdometer spawns for it is the official `claude` binary. The webview
receives only usage percentages and reset times over
one-way events, runs under a strict CSP, has no invokable backend commands,
and no filesystem or shell capabilities. (The webview also receives a single
`working` boolean for the activity animation, and — only while an Audiowave Orb
[skin](../README.md#skins) is selected — a stream of 36 audio spectrum magnitudes for the
ring. The backend derives both locally; neither carries usage data, and the
spectrum bands are bar heights, not audio: nothing is recorded or stored.)

For that activity flag the HUD also **reads** (never writes) Claude Code's
transcript files under `~/.claude/projects/`, to tell when a session is
generating. It inspects only the last message's turn state locally — no network,
nothing leaves the process.

This design is deliberate: Anthropic's Consumer Terms of Service prohibit
using OAuth tokens from Claude Free/Pro/Max accounts in any third-party tool,
so Clawdometer only consumes what Claude Code itself hands to its documented
statusline interface — nothing is impersonated, nothing extra is requested.

**Don't trust — verify.** Everything above is checkable in this repository:

- `cargo deny check bans` proves no HTTP/TLS crate is compiled into any
  binary (config in [`deny.toml`](../deny.toml)) — and the source contains no
  network code to begin with (`grep` it: no sockets, no curl, no URLs).
- The webview's entire permission set is
  [`app/src-tauri/capabilities/default.json`](../app/src-tauri/capabilities/default.json)
  — events and window-dragging, nothing else.
- Release binaries are **not code-signed**. If you don't want to trust a
  downloaded binary, build from source (below) — it takes one command.
  Only download releases from this repository's official Releases page;
  a binary from anywhere else could be a tampered copy.

**Writes:** only `~/.clawdometer/` and the `statusLine` key of
`~/.claude/settings.json` (during `install`/`uninstall`, or the HUD's
one-time auto-install when no statusline exists — a full backup is taken
first either way). Exception: the tray's "Start with Windows" toggle writes
the standard HKCU Run registry key, only when you click it.

**Two things worth knowing:**

- `clawdometer install` saves a full backup of your `settings.json` to
  `~/.clawdometer/backups/` before touching it. If your settings contain
  secrets (an `env` block with API keys, an `apiKeyHelper` command), those
  are in the backups too — delete `~/.clawdometer/backups/` when you no
  longer need them, or use `uninstall --purge`.
- Avoid running `install`/`uninstall` while a Claude Code session is actively
  changing settings — both edit `settings.json`, and the last writer wins.

## Requirements

- Windows 10 or 11.
- Microsoft Edge WebView2 runtime — preinstalled on Windows 11 and updated
  Windows 10. If missing, the installer downloads it, which needs internet.
- Claude Code (CLI) installed and signed in — all data comes from its statusline
  hook, so the HUD only updates while Claude Code is in use. (Clawdometer
  honors `%CLAUDE_CONFIG_DIR%` if you've relocated `~/.claude`.)

## Getting started

1. **Run the HUD** (`Clawdometer.exe`). A tray icon appears and the HUD
   window shows up. If Claude Code has no statusline yet, the HUD claims it
   automatically (one-time, settings backed up first) — no CLI step
   required. Use Claude Code and the percentages appear with every response,
   with your statusline showing `[Model] 5h X% · 7d Y%`. Launching the HUD a
   second time just brings the existing one to the front (single instance).
2. **Only if you already have a custom statusline:** run
   `clawdometer install` in a terminal. This chains your original statusline
   (its output still renders) while also feeding the HUD.

## HUD usage

- **Move it:** drag the card anywhere; the position is saved once the drag
  settles and remembered across restarts (and sanity-checked against your
  current monitors, so an unplugged display can't strand it off-screen).
- **Tray icon, left-click:** show/hide the HUD.
- **Tray icon, right-click:** menu with *Show/Hide*, *Refresh usage* (runs a
  headless `claude /usage` now), *RICE*, *Compact size*, *Fable bar*, *Opacity*,
  *Start with Windows* (check mark reflects the actual HKCU Run key state), and
  *Quit*.
- **RICE:** picks the skin — *Classic*, *Rowline*, *Bento Box*, *ClawdBoy* →
  *Table* / *Menu* / *Quad*, or *Audiowave
  Orb* → *Bars* / *Peak hold* / *LED Bloom* (see [Skins](../README.md#skins)). One radio group, so exactly
  one is ever checked. Switching resizes the HUD (the orb is a 160×160 square;
  each card skin has its own size) and is remembered across restarts. Selecting
  an orb skin starts the system-audio loopback capture; going back to a card
  skin stops it.
- **Compact size:** shrinks the card to roughly half width (the same numbers,
  with 5H / 7D / FBL row labels on Rowline). Also toggled by
  double-clicking the card. Remembered across restarts.
- **Fable bar:** on by default. Fable's weekly window is a Max-plan limit, so on
  any other plan that third row can never fill — turn it off and every card skin
  re-cuts around two windows: the row stacks shorten, Bento widens Weekly into
  the freed cell, and ClawdBoy keeps its shell and spends the freed line on
  bigger type. Remembered across restarts.
- **Opacity:** 100/85/70/55% — makes the always-on-top card less visually
  blocking. Also available by right-clicking the card. Remembered across
  restarts.
- **Activity animation:** while any Claude Code session is generating, the clawd
  mascot plays a "working" animation — one picked at random each turn from a pool
  of little motions (a worker bob, hops, a march, clapping, a jelly wobble, and
  more), drawn without repeats so it rarely does the same thing twice in a row.
  It rests when everything is idle. Unlike the usage numbers, this doesn't depend
  on the statusline hook (so it works even in GUI clients that don't run it): the
  HUD reads the turn state from Claude Code's transcript files under
  `~/.claude/projects/` — locally, no network — and stays lit through long
  "thinking" pauses, stopping the moment a turn completes. Respects
  `prefers-reduced-motion` (the mascot holds still, no motion).
- **Empty state:** before any data has ever arrived, the percentages show as
  "—" and the tray tooltip reads "waiting for first usage refresh" — the
  headless refresher fetches within about a minute of launch. When a limit
  window's reset time passes, the HUD shows 0% for it on its own (derived
  locally, no network).

## CLI

```
clawdometer install      # backs up settings.json, sets/wraps statusLine
clawdometer status       # print the latest snapshot + capture time
clawdometer uninstall    # restores the original statusLine (or removes the key)
clawdometer uninstall --purge   # also deletes ~/.clawdometer/
```

- `install` writes a timestamped backup of your `settings.json` to
  `~/.clawdometer/backups/` before touching anything, and never overwrites
  an existing backup.
- `install` is idempotent; re-running after moving the binary updates the
  stale path in place.
- If you edited `statusLine` yourself after installing, `uninstall` refuses
  to touch it and tells you where your original is preserved.
- `--settings <path>` (for `install`/`uninstall`) targets a non-default
  settings.json — mainly for testing.
- **Complete removal:** run `clawdometer uninstall --purge`, then uninstall
  the HUD app from Windows *Apps & features* — its uninstaller also removes
  the "Start with Windows" autostart entry. If you run a portable or
  from-source build (no installer), toggle *Start with Windows* off in the
  tray menu **before** deleting the binary; otherwise the HKCU Run registry
  value it created is left pointing at a deleted exe.

## Files

Everything lives in `~/.clawdometer/`:

| File | Purpose |
|------|---------|
| `state.json` | last statusline snapshot (written by the hook) |
| `live.json` | last refresh snapshot (written by the HUD after a headless `claude /usage` run) |
| `wrapped.json` | your original statusline command, chained + restored on uninstall |
| `ui.json` | HUD window position, opacity, compact mode |
| `statusline-autoinstall.done` | marker: the HUD offered to claim a free statusLine once (so removing it later sticks) |
| `backups/` | timestamped copies of settings.json taken before each install (may contain secrets from your settings — see Security) |

## Building from source

See "Install" in the [README](../README.md#install). Additionally:

```
cargo test --workspace     # full test suite
cargo deny check bans      # verify the no-network-crates invariant
```

## Notes

- Percentages have 1% granularity — the same as `/usage` inside Claude Code.
- Data updates with every Claude Code response; when that goes quiet the HUD
  refreshes via a headless `claude /usage` about once a minute, so usage
  made on claude.ai web/mobile still shows up. It also zeroes a window
  locally once its reset time passes.
- Clawdometer's own binaries are fully offline: proxies, VPNs, and firewalls
  only matter to Claude Code itself.
