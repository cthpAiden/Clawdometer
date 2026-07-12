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
> shells out to Windows' bundled `curl.exe`).
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
`~/.clawdometer/state.json` — that's where the HUD's model name comes from.
Whichever source is newer wins.

If you already had a statusline configured, Clawdometer preserves it and
chains it: your original statusline still renders its output (with a 2-second
timeout), and `uninstall` restores it exactly.

## Install / uninstall

```
clawdometer install      # backs up settings.json, sets/wraps statusLine
clawdometer uninstall    # restores the original statusLine (or removes the key)
clawdometer uninstall --purge   # also deletes ~/.clawdometer/
```

`install` writes a timestamped backup of your `settings.json` to
`~/.clawdometer/backups/` before touching anything.

## Notes

- Percentages have 1% granularity — the same as `/usage` inside Claude Code.
- The HUD footer shows how old the data is ("as of Xm ago"). With live
  polling working it should never say more than a minute; if it grows, the
  poll is failing (no network, or the OAuth token expired — using Claude Code
  once refreshes the token).

## License

MIT
