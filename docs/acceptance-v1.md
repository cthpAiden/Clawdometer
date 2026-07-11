# v1 Manual Acceptance — 2026-07-12

Automated gates (verified this date, `clawdometer-v1` branch):

- [x] `cargo test --workspace` green (55 tests, 0 failed)
- [x] `cargo deny check bans` green (network crates banned)
- [x] `cargo build --release` produces `clawdometer.exe` and the Tauri HUD app
- [x] E2E lifecycle test (`crates/cli/tests/e2e_test.rs`): install into sandbox
      settings.json → run the exact installed command via `cmd /C` with both
      real fixtures → state.json correct → chained statusline output preserved
      → uninstall restores deep-equal

Manual checklist (run on the dev machine against the REAL `~/.claude/settings.json`;
check off with observed values):

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
