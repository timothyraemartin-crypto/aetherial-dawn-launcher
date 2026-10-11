# Progress (source of truth - keep short, newest first)

Read this at the start of every task; update it as you fix or build things.
Protocol v2.0.0: `.claude/ad-system/OPERATING_PROTOCOL.md`. Checks: `python3 .claude/ad-system/hook.py verify --session ID [--check core|memory-lint]` (core = cargo check -p launcher-core --tests), then `handoff`, `gate`.

## Now
- 2026-10-11 Bundled AetherialSkillTree 0.1.2 (sha256 E1F0CD14AA0033FC4D6D6A45AEA0440BA35AAA9778641B8C318C16ABD0812225) replacing 0.1.1: 0.1.1 went idle at start because BSString::Set_CStr (id 11044) is not in versionlib-1-6-1170-0.bin; 0.1.2 uses the game allocator and makes each hook optional. Source in native-mods/skill-tree. Checked: offline address-id check and parse/data tests pass, artifact sha verified; NOT runtime verified in game.
- 2026-10-11 Book bug (draft PR claude/book-menu-break-y5k7pc): launcher now writes AetherialDawn-NoPause.ini with Book=0 (write-once) so books keep the vanilla pause; NoPause DLL unchanged. Implemented only: cargo test not run here (no gdk system libs), not built, merged, released or game-verified. Cause is INFERRED, symptom (freeze/crash/stuck) UNKNOWN. Status NEEDS_GAME_CHECK: checklist step 8 in native-mods/nopause/NOPAUSE-CHECKLIST.md.
- 2026-10-11 build.yml `windows` job moved to GitHub-hosted windows-latest (PC runner offline, Timothy's call) so the v0.1.134 release does not wait on his PC. Checks: manual dispatch of Build launcher on the PR branch. Not released yet; tag v0.1.134 follows the merge.
- 2026-10-11 Branch mod-removal (0.1.134, 22 commits) pushed as a draft PR with native-mods/ sources. Share build 0.1.135 (the share folder under A:/Aetherial Dawn) has no matching source commit: Cargo.toml and tree are at 0.1.134. Not released, not deployed.

## Done
- 2026-10-11 Removed 6 unreferenced files (ui/art/background-dawn.jpg, ui/art/icon-128.png, ui/art/README.md, docs/connect-vortex-runbook.md, docs/publish-feed.md, core/examples/gamever.rs). Checked: cargo check -p launcher-core --tests, node tests, 3 UI tests. Not built as Windows app; no game check needed.
- 2026-10-08 Added repo memory (`.claude/memory/`: INDEX.md + topic files, linted by `.claude/memory-lint.sh`).
- 2026-10-08 Installed Aetherial Dawn Hook v2.0.0 (package installer; `.claude/ad-system/`, config `.claude/ad-hook.json`), replacing `.claude/progress-hook.sh` and its settings entries. Hook commands use $CLAUDE_PROJECT_DIR.

## Broken / Next
- (none known)
