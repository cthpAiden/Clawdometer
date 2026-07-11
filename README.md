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

## What it does

Claude Code sends usage data (5-hour and 7-day rate-limit percentages, reset
times, context window usage) to your statusline command on every API response.
Clawdometer installs itself as that statusline command, records the latest
snapshot to `~/.clawdometer/state.json`, and shows it in a small always-on-top
HUD and a system-tray tooltip (`5h X% · 7d Y%`).

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
- Data appears only after Claude Code fires the statusline (the first API
  response of a session). Some surfaces never fire it and will show nothing.
- The HUD footer shows how old the data is ("as of Xm ago") once a session
  ends.

## License

MIT
