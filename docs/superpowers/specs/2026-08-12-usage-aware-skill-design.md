# usage-aware — design spec

**Date:** 2026-08-12
**Status:** approved for planning
**Deliverable:** standalone Claude Code skill + SessionStart hook. Installs to `~/.claude/`. NOT part of the Clawdometer app; spec lives in this repo because it is the working workspace and the skill opportunistically reads Clawdometer's `live.json`.

## Problem

Claude Code has no awareness of the account's rate-limit windows (5-hour session, 7-day all-models, 7-day Fable). It burns the weekly budget on speculative subagents, whole-file reads, and broad refactors even when the user is at 90% with three days until reset. The user wants Claude to *adapt its own behavior* to remaining budget.

## Solution overview

Two artifacts, no daemon, no build step:

```
~/.claude/skills/usage-aware/SKILL.md    # tier rules (reference, model-facing)
~/.claude/hooks/usage-tier.ps1           # SessionStart hook (Windows)
~/.claude/hooks/usage-tier.sh            # SessionStart hook (POSIX)
```

Plus a `SessionStart` entry in `~/.claude/settings.json` invoking the platform script.

### Data flow

```
source 1: ~/.clawdometer/live.json      (Clawdometer HUD 60s poll; free when present)
source 2: ~/.claude/usage-aware/cache.json  (skill's own cache)
source 3: detached `claude -p --no-session-persistence /usage`  (writes source 2)

SessionStart hook -> pick freshest valid source -> print one line into context:
  [usage] week 39% · session 32% · week resets Jul 16, 9:59am · tier: normal
Claude reads tier -> adapts per SKILL.md rules.
```

### Source priority (hook logic)

1. `~/.clawdometer/live.json` exists AND mtime < 15 min → parse `rate_limits.seven_day.used_percentage` (and `five_hour`), print line with computed tier. Done.
2. Else `~/.claude/usage-aware/cache.json` exists AND mtime < 10 min → same. Done.
3. Else: spawn detached refresher (below), and:
   - if a stale cache/live.json exists (< 24 h), print numbers tagged `(stale)` with NO tier;
   - else print nothing.
   Exit 0 in all cases.

### Detached refresher

Runs `claude -p --no-session-persistence /usage`, parses stdout, writes cache.

- **Recursion guard:** refresher child is launched with env `USAGE_AWARE_REFRESH=1`. The SessionStart hook exits immediately (before any file I/O) when that var is set. Without this, the child's own startup fires the hook, sees a stale cache, spawns a grandchild — unbounded.
- **Pile-up guard:** lockfile `~/.claude/usage-aware/refresh.lock` containing PID. Skip spawn if lockfile mtime < 5 min. Stale lock (≥ 5 min) is overwritten. (Clawdometer precedent: hung `/usage` children accumulate at one per poll; see `app/src-tauri/src/usage_refresher.rs` kill_tree comment.)
- **Timeout:** refresher kills its `claude` child after 60 s and exits without writing.
- **Atomic write:** cache written to `cache.json.tmp` then renamed. Prevents torn reads by a concurrent session's hook.
- **Windows detach:** `Start-Process -WindowStyle Hidden`; POSIX: `nohup ... &` with stdio to /dev/null.

### Parsing

Only two lines matter (format captured from CLI v2.1.207, same as Clawdometer's tested parser):

```
Current session: 32% used · resets Jul 13, 3:29pm (Asia/Saigon)
Current week (all models): 39% used · resets Jul 16, 9:59am (Asia/Saigon)
```

Extract integer before `%` and the reset string between `resets ` and ` (`. Reset string is kept verbatim for display — no timestamp math (skill shows "resets Jul 16, 9:59am" as-is; tier only needs the percentage). If the week line fails to parse, treat run as failed; keep old cache. Session line optional.

Cache shape (subset of Clawdometer's State, but independent):

```json
{
  "fetched_at": "2026-08-12T02:31:00Z",
  "week_pct": 39,
  "session_pct": 32,
  "week_resets": "Jul 16, 9:59am",
  "session_resets": "Jul 13, 3:29pm"
}
```

live.json uses Clawdometer's schema (`rate_limits.seven_day.used_percentage`, epoch `resets_at`); hook handles both shapes. Epoch → human display: formatted by the hook script in local time.

### Tiers (weekly % drives; SKILL.md content)

| Week % | Tier | Rules |
|---|---|---|
| < 60 | normal | no behavior change |
| 60–85 | frugal | no subagents/workflows unless user asks; targeted reads (offset/limit, Grep) over whole-file reads; no speculative refactors or drive-by cleanups; prefer editing over regenerating |
| > 85 | minimal | smallest change that works; confirm with user before any subagent, workflow, or > ~500-line read; no exploratory searches beyond what the task strictly needs |

Skill also instructs: when a long/expensive task starts mid-session and the hook line is > 1 h old, re-read the freshest of live.json/cache.json before deciding scope (Read tool, not a new `/usage` run).

Session (5-hour) % is displayed but never drives tier — it resets too fast; throttling on it over-corrects.

### Failure modes — all fail open

| Condition | Behavior |
|---|---|
| no live.json, no cache | spawn refresher, print nothing |
| both stale (> 24 h) | spawn refresher, print stale line, no tier |
| malformed JSON either source | treat as absent |
| `claude` not on PATH in refresher | exit silently, lockfile mtime prevents retry storm for 5 min |
| `USAGE_AWARE_REFRESH=1` set | exit 0 immediately |

No tier line ⇒ Claude behaves normally. Missing data must never throttle.

### Explicitly out of scope (YAGNI)

- Multi-account support (single login confirmed)
- Fable-week gating (weekly all-models only)
- Push notifications / threshold alerts (Clawdometer HUD's job)
- Any change to the Clawdometer app or installer
- Reset-time timestamp math / year rollover (display verbatim strings; only Clawdometer needs epochs)

## Testing

Hook scripts are pure functions of (env, two files, clock) → stdout. Test via a fixture dir and `CLAWDOMETER_DIR`-style overrides: hook honors `USAGE_AWARE_DIR` (cache/lock location) and `CLAWDOMETER_DIR` (live.json location) for tests.

Cases (Pester for .ps1, bats or plain sh-assert for .sh):

1. tier boundaries: week 59→normal, 60→frugal, 85→frugal, 86→minimal
2. live.json fresh beats cache.json fresh (priority)
3. live.json stale + cache fresh → cache wins
4. both stale → stale line, no tier, refresher spawned (assert lockfile created; stub `claude` on PATH with a fake script)
5. both missing → silence, refresher spawned
6. malformed live.json + good cache → cache used
7. `USAGE_AWARE_REFRESH=1` → instant exit, no output, no spawn
8. lockfile fresh → no second spawn
9. refresher: fake `claude` printing captured report → cache.json written atomically with week_pct 39
10. refresher: fake `claude` printing garbage → no cache write, old cache intact

## Implementation notes for the planner

- PowerShell 5.1 compatible (no `&&`, no ternary) — see repo CLAUDE.md conventions.
- SKILL.md frontmatter `description` must trigger on: "how much usage left", "check my limits", "am I close to my limit", and instruct always-loaded behavior is NOT expected — the hook line is the always-on part.
- settings.json edit: additive merge into existing `hooks.SessionStart` array, never clobber. Provide an install script `install.ps1` (separate from Clawdometer's) that does this merge with a backup.
- Verbatim `/usage` report fixture: copy from `app/src-tauri/src/usage_refresher.rs` test constant.
