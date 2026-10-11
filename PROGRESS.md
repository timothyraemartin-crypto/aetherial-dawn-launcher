# Progress (source of truth - keep short, newest first)

Read this at the start of every task; update it as you fix or build things.
Protocol v2.0.0: `.claude/ad-system/OPERATING_PROTOCOL.md`. Checks: `python3 .claude/ad-system/hook.py verify --session ID [--check core|memory-lint]` (core = cargo check -p launcher-core --tests), then `handoff`, `gate`.

## Now
- 2026-10-11 Branch claude/force-update-in-game (on top of #117): a launcher release published while Skyrim runs shows a 60 s countdown (window raised), ends Skyrim (new Tauri commands close_game_for_update, raise_launcher), installs and relaunches. Kill switch: status.json launcher.forceClose=false; launcher.forceCloseSecs 5-300. Text: "Launcher X is ready. Skyrim closes in N seconds to install it. Save your game." (needs Timothy's OK). Checked: ui-fast-play-test (2 new scenarios), ui-contract-test. src-tauri NOT compiled here (no Windows/GTK toolchain); Windows build and in-game visibility of the raised window over fullscreen NEED game check. Not released.
- 2026-10-11 Play now checks for a newer launcher release first (ui/app.js updateBeforePlay): installs and relaunches before the game starts, 10 s timeout, offline/failed check logs and starts with the current version, staff holds still apply. New text: "Updating the launcher to X before the game starts…" (needs Timothy's OK before merge). Checked: ui-fast-play-test (3 new scenarios), ui-contract-test. Not built as Windows app; real Play-time update NEEDS_GAME_CHECK. Not released.
- 2026-10-11 build.yml `windows` job moved to GitHub-hosted windows-latest (PC runner offline, Timothy's call) so the v0.1.134 release does not wait on his PC. Checks: manual dispatch of Build launcher on the PR branch. Not released yet; tag v0.1.134 follows the merge.
- 2026-10-11 Branch mod-removal (0.1.134, 22 commits) pushed as a draft PR with native-mods/ sources. Share build 0.1.135 (the share folder under A:/Aetherial Dawn) has no matching source commit: Cargo.toml and tree are at 0.1.134. Not released, not deployed.

## Done
- 2026-10-11 Removed 6 unreferenced files (ui/art/background-dawn.jpg, ui/art/icon-128.png, ui/art/README.md, docs/connect-vortex-runbook.md, docs/publish-feed.md, core/examples/gamever.rs). Checked: cargo check -p launcher-core --tests, node tests, 3 UI tests. Not built as Windows app; no game check needed.
- 2026-10-08 Added repo memory (`.claude/memory/`: INDEX.md + topic files, linted by `.claude/memory-lint.sh`).
- 2026-10-08 Installed Aetherial Dawn Hook v2.0.0 (package installer; `.claude/ad-system/`, config `.claude/ad-hook.json`), replacing `.claude/progress-hook.sh` and its settings entries. Hook commands use $CLAUDE_PROJECT_DIR.

## Broken / Next
- (none known)
