# Progress (source of truth - keep short, newest first)

Read this at the start of every task; update it as you fix or build things.
Protocol v2.0.0: `.claude/ad-system/OPERATING_PROTOCOL.md`. Checks: `python3 .claude/ad-system/hook.py verify --session ID [--check core|memory-lint]` (core = cargo check -p launcher-core --tests), then `handoff`, `gate`.

## Now
- Vortex removal, slice A (draft PR): docs, CI steps, review notes removed. `vortex-extension/` stays until the mod-removal lead PR deletes core/src/vortex.rs (it include_bytes! the extension); then delete the dir. Not built or run; no Rust changed.

## Done
- 2026-10-08 Added repo memory (`.claude/memory/`: INDEX.md + topic files, linted by `.claude/memory-lint.sh`).
- 2026-10-08 Installed Aetherial Dawn Hook v2.0.0 (package installer; `.claude/ad-system/`, config `.claude/ad-hook.json`), replacing `.claude/progress-hook.sh` and its settings entries. Hook commands use $CLAUDE_PROJECT_DIR.

## Broken / Next
- (none known)
