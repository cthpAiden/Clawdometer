# Clawdometer

Unofficial Windows desktop HUD for Claude Code usage limits.

> **Unofficial.** Not affiliated with or endorsed by Anthropic.
> **Network:** the HUD makes exactly one kind of outbound request — a
> read-only GET to Anthropic's official usage endpoint
> (`api.anthropic.com/api/oauth/usage`, the same source `/usage` reads),
> every 60 seconds, authenticated with the OAuth token Claude Code already
> stores in `~/.claude/.credentials.json`. The token is never sent anywhere
> else and never put on a command line. The CLI/hook remain fully offline
> (enforced via a cargo-deny ban on network crates; the HUD's single request
> shells out to Windows' bundled `curl.exe`, invoked by absolute
> `System32` path).
> **Writes:** only `~/.clawdometer/` and the `statusLine` key of
> `~/.claude/settings.json` (during `install`/`uninstall`). Exception: the
> tray's "Start with Windows" toggle writes the standard HKCU Run registry
> key, only when you click it.

## What it does

The HUD polls Anthropic's usage endpoint every 60 seconds, so the 5-hour and
7-day rate-limit percentages stay fresh even when no Claude Code session is
running. It shows them in a small always-on-top HUD and a system-tray tooltip
(`5h X% · 7d Y%`).

Additionally, Claude Code sends usage data (percentages, reset times, model,
context window) to your statusline command on every API response. Clawdometer
installs itself as that statusline command and records the latest snapshot to
`~/.clawdometer/state.json`. Whichever source is newer wins — in the HUD and
in `clawdometer status`.

The HUD header shows a countdown to the 5-hour window reset (limits are
account-wide, so a model name would add nothing). The footer shows data age
and turns red with a hint if polling has been failing for over 10 minutes.

If you already had a statusline configured, Clawdometer preserves it and
chains it: your original statusline still renders its output (with a 2-second
timeout), and `uninstall` restores it exactly.

## Requirements

- Windows 10 1803+ (needs the bundled `curl.exe`) or Windows 11.
- Claude Code installed and signed in (the HUD reads its OAuth token from
  `~/.claude/.credentials.json`; using Claude Code refreshes it).

## Getting started

1. **Run the HUD** (`Clawdometer.exe`). A tray icon appears and the HUD
   window shows up. Within a minute it displays live percentages — no CLI
   step required. Launching it a second time just brings the existing HUD to
   the front (single instance).
2. **Optional — statusline integration:** run `clawdometer install` in a
   terminal. This sets Clawdometer as your Claude Code statusline command, so
   every Claude Code response also updates the HUD instantly and your
   statusline shows `[Model] 5h X% · 7d Y%`.

## HUD usage

- **Move it:** drag the card by its title/background; the position is
  remembered across restarts (and sanity-checked against your current
  monitors, so an unplugged display can't strand it off-screen).
- **Tray icon, left-click:** show/hide the HUD.
- **Tray icon, right-click:** menu with *Show/Hide*, *Start with Windows*
  (toggles the HKCU Run key), and *Quit*.
- **Footer:** data age ("as of 1m ago"). If it turns red saying the poll is
  failing, your network is down or the OAuth token expired — using Claude
  Code once refreshes it.

## CLI

```
clawdometer install      # backs up settings.json, sets/wraps statusLine
clawdometer status       # print the current merged snapshot + capture time
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

## Files

Everything lives in `~/.clawdometer/`:

| File | Purpose |
|------|---------|
| `state.json` | last statusline snapshot (written by the hook) |
| `live.json` | last poller snapshot (written by the HUD every 60s) |
| `wrapped.json` | your original statusline command, chained + restored on uninstall |
| `ui.json` | HUD window position |
| `backups/` | timestamped copies of settings.json taken before each install |

## Building from source

Rust (MSVC toolchain, pinned via `rust-toolchain.toml`) and
[tauri-cli](https://tauri.app) are required.

```
cargo build --release -p clawdometer-cli   # -> target/release/clawdometer.exe
cd app/src-tauri && cargo tauri build      # -> HUD app + NSIS installer
cargo test --workspace                     # full test suite
```

## Notes

- Percentages have 1% granularity — the same as `/usage` inside Claude Code.
- The HUD footer shows how old the data is ("as of Xm ago"). With live
  polling working it should never say more than a minute; if it grows, the
  poll is failing (no network, or the OAuth token expired — using Claude Code
  once refreshes the token).

## License

MIT
