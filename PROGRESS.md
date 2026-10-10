# Progress (source of truth - keep short, newest first)

Read this at the start of every task; update it as you fix or build things.
Protocol v2.0.0: `.claude/ad-system/OPERATING_PROTOCOL.md`. Checks: `python3 .claude/ad-system/hook.py verify --session ID [--check core|memory-lint]` (core = cargo check -p launcher-core --tests), then `handoff`, `gate`.

## Now
- 2026-10-10 Server-feed mod removal (draft, hold until the mod-free launcher is released): publish-feed.sh/workflow modes `strip-mods-preview` and `strip-mods` (empty list, sign, move hosted mod files to backup). `bash scripts/test-publish-feed.sh` passes. NOT run on the VPS.

## Done
- 2026-10-08 Added repo memory (`.claude/memory/`: INDEX.md + topic files, linted by `.claude/memory-lint.sh`).
- 2026-10-08 Installed Aetherial Dawn Hook v2.0.0 (package installer; `.claude/ad-system/`, config `.claude/ad-hook.json`), replacing `.claude/progress-hook.sh` and its settings entries. Hook commands use $CLAUDE_PROJECT_DIR.

## Broken / Next
- (none known)
