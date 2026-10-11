# Progress (source of truth - keep short, newest first)

Read this at the start of every task; update it as you fix or build things.
Protocol v2.0.0: `.claude/ad-system/OPERATING_PROTOCOL.md`. Checks: `python3 .claude/ad-system/hook.py verify --session ID [--check core|memory-lint]` (core = cargo check -p launcher-core --tests), then `handoff`, `gate`.

## Now
- 2026-10-11 Branch mod-removal (0.1.134, 22 commits) pushed as a draft PR with native-mods/ sources. Share build 0.1.135 (the share folder under A:/Aetherial Dawn) has no matching source commit: Cargo.toml and tree are at 0.1.134. Not released, not deployed.

## Done
- 2026-10-11 Removed 6 unreferenced files (ui/art/background-dawn.jpg, ui/art/icon-128.png, ui/art/README.md, docs/connect-vortex-runbook.md, docs/publish-feed.md, core/examples/gamever.rs). Checked: cargo check -p launcher-core --tests, node tests, 3 UI tests. Not built as Windows app; no game check needed.
- 2026-10-08 Added repo memory (`.claude/memory/`: INDEX.md + topic files, linted by `.claude/memory-lint.sh`).
- 2026-10-08 Installed Aetherial Dawn Hook v2.0.0 (package installer; `.claude/ad-system/`, config `.claude/ad-hook.json`), replacing `.claude/progress-hook.sh` and its settings entries. Hook commands use $CLAUDE_PROJECT_DIR.

## Broken / Next
- (none known)
