---
name: quality-reviews-4
description: Open findings from the launcher Tauri shell review (src-tauri/) and the UI/command contract; last of ten reviews
type: reference
verified: 2026-10-07
refs: src-tauri/src/main.rs, src-tauri/src/mods.rs, ui/app.js, .github/workflows/build.yml
---
Read-only review on 2026-10-07 (container lacked GTK/WebKit, so src-tauri's 4 unit tests were not run); no code changed. Verdict: solid and careful, but untested glue and lots of dead code. Report: https://claude.ai/artifact/GePGALbMCxBvW2zNBVxxCG

- CI runs only `cargo test -p launcher-core`, so src-tauri tests, clippy and fmt never run; src-tauri tests do not compile on main (two VortexMod initializers in `mods.rs` tests lack `file_md5`).
- FIX IN DRAFT PR #57 (not merged): removes the unreachable download queue, the Nexus sign-in commands (incl. clipboard), `cancel_mods`, `game_check`, `last_game_report` and the clipboard plugin; mods.rs 1352 -> 501 lines; README says only the patch route ships. Open: `export.rs` (staff server-lane export) still reads a saved Nexus key nothing can save any more. Removal candidates left for Timothy: Steam downgrader, Pandora (`tools.rs`), Nexus sso/clipboard/nxm helpers in `nexus.rs`, dead `music.rs` items.
- FIX IN DRAFT PR #55 (not merged): Play no longer waits on the pre-Play staff report upload (own task, still completes); `aetherial-collection.json` is fetched once per Play; `config.json` is written atomically and a damaged one is copied to `config.json.bad` before defaults load. Still open: the Play sequence has no unit tests (src-tauri tests are not in CI).
- UI contract: all 35 commands, 7 events and the error prefixes (`NEEDS_NEXUS_MODS:`, `VORTEX_NOT_READY:`, `SIGNED_OUT:`, `NO_PATCH:`, `NO_PATCH_FILES:`) match `ui/app.js` today, but nothing enforces it.

**How to apply:** before renaming a command, event or error prefix, change `ui/app.js` in the same piece of work; consider a contract test.
