# Progress (source of truth - keep short, newest first)

Read this at the start of every task; update it as you fix or build things.
Build check (run by the Stop hook): `cargo check -p launcher-core --tests` when Rust changed, plus `node --check` on changed JS.

## Now
- Dead-file cleanup is open as draft PR #51 (removes background-dawn.jpg, icon-128.png, connect-vortex-runbook.md).

## Done
- 2026-10-07 Added repo memory (`.claude/memory/`: INDEX.md + topic files, linted by `.claude/memory-lint.sh`).
- 2026-10-07 Added progress/build hooks (`.claude/progress-hook.sh`).

## Broken / Next
- vortex-extension robustness (null body hangs, unguarded startup I/O, token read once, no index.js test): see memory topic vortex-extension-review.
- Launcher calls sign-in/status/crash endpoints missing from discord main; plugins.txt can be wiped on a non-UTF-8 plugin name: see memory topic quality-reviews (CONFIRMED: force_on/switch_on in loadorder.rs rewrite plugins.txt empty on an ANSI read failure, no backup).
- File sync: unsigned manifest, unscoped `remove`, no retry on failed download: see memory topic quality-reviews-2.
- Unsigned mods.json; Vortex Ready can be faked locally; Steam downgrader unused: see memory topic quality-reviews-3.
