# Progress (source of truth - keep short, newest first)

Read this at the start of every task; update it as you fix or build things.
Build check (run by the Stop hook): `cargo check -p launcher-core --tests` when Rust changed, plus `node --check` on changed JS.

## Now
- Draft PR #56: launcher/server contract CI (`contract/`, `contract.yml`, Rust sign-in test). Draft PR #57: dead code removal (download queue, Nexus sign-in/clipboard, cancel_mods, game_check, last_game_report); mods.rs 1352 -> 501 lines.
Fix threads started 2026-10-07 from the review findings (see memory quality-reviews*; update/delete a finding when its fix merges):
- plugins.txt data loss: fix in draft PR #54 (CI pending; not merged; untested on Windows)
- sign the server mods.json and manifest: draft PR #58 (CI pending; not merged; server steps for Timothy in memory signed-feeds)
- src-tauri CI (tests, clippy, fmt)
- remove dead launcher code (unreachable Download-all-mods queue)
- Play stall on staff report upload: fix in draft PR #55 (CI pending; not merged)
- Dead-file cleanup is open as draft PR #51 (removes background-dawn.jpg, icon-128.png, connect-vortex-runbook.md).

## Done
- 2026-10-07 Added repo memory (`.claude/memory/`: INDEX.md + topic files, linted by `.claude/memory-lint.sh`).
- 2026-10-07 Added progress/build hooks (`.claude/progress-hook.sh`).

## Broken / Next
- vortex-extension robustness (null body hangs, unguarded startup I/O, token read once, no index.js test): see memory topic vortex-extension-review.
- Launcher calls sign-in/status/crash endpoints missing from discord main; plugins.txt can be wiped on a non-UTF-8 plugin name: see memory topic quality-reviews (CONFIRMED: force_on/switch_on in loadorder.rs rewrite plugins.txt empty on an ANSI read failure, no backup).
- File sync: unsigned manifest, unscoped `remove`, no retry on failed download: see memory topic quality-reviews-2.
- Unsigned mods.json (fix in #58); Vortex Ready can be faked locally; Steam downgrader unused: see memory topic quality-reviews-3.
- Tauri shell: dead code removal is draft PR #57; CI skips src-tauri tests/clippy/fmt, and src-tauri tests do not compile on main (missing `file_md5` in two VortexMod initializers in mods.rs tests): see memory topic quality-reviews-4.
