---
name: quality-reviews-4
description: Open findings from the launcher Tauri shell review (src-tauri/) and the UI/command contract; last of ten reviews
type: reference
verified: 2026-10-07
refs: src-tauri/src/main.rs, src-tauri/src/mods.rs, ui/app.js, .github/workflows/build.yml
---
Read-only review on 2026-10-07 (container lacked GTK/WebKit, so src-tauri's 4 unit tests were not run); no code changed. Verdict: solid and careful, but untested glue and lots of dead code. Report: https://claude.ai/artifact/GePGALbMCxBvW2zNBVxxCG

- FIX IN DRAFT PR #59 (not merged, Windows CI running): adds src-tauri tests, clippy `-D warnings` on both crates and a UI/command contract check (`.github/scripts/ui-contract-test.js`, 35 commands and 7 events); fixes the src-tauri test compile break (missing `file_md5`), 4 tests pass. Temporary `#[allow(dead_code)]` on `mod mods;` (the dead-code thread removes it). rustfmt is report-only (~1,600 spots): after the in-flight fix PRs land, run `cargo fmt --all` once and drop `continue-on-error`.
- FIX IN DRAFT PRs (not merged): #57 (green incl. Windows Tauri build) removes the unreachable download queue and 9 unused Tauri commands (Nexus sign-in incl. clipboard, `cancel_mods`, `game_check`, `last_game_report`, clipboard plugin; mods.rs 1352 -> 501 lines; README says only the patch route ships). #60 (stacked on #57, green; merge #57 first, then retarget #60 to main) removes the Steam downloader (`downgrade.rs`, `steamapp.rs`), unused Nexus code, the `game.tool` manifest field and sign-in-only websocket/TLS crates (~1,000 lines), and adds Settings > "Staff: server-mod export" for a Nexus Premium key (validated with Nexus, stored encrypted in `nexus.bin`, used only by the export, never logged). Corrections to the removal list: Pandora was only a doc comment in `tools.rs` (the allow-list is BodySlide only); `music.rs` items are used on Windows, so kept. Open: retarget #60 to main once #57 merges; whichever of #59 and #60 merges second drops #59's `#[allow(dead_code)]`; the `nxm://` restore code is gone, so a crashed pre-0.1.68 build could still leave nxm links pointing at the launcher; #58 test-merges cleanly with #57.
- FIX IN DRAFT PR #55 (CI green, ready for review, not merged): Play no longer waits on the pre-Play staff report upload (own task, still completes); `aetherial-collection.json` is fetched once per Play; `config.json` is written atomically and a damaged one is copied to `config.json.bad` before defaults load. Still open: the Play sequence has no unit tests (src-tauri tests are not in CI).
- UI contract (now checked by #59's script): all 35 commands, 7 events and the error prefixes (`NEEDS_NEXUS_MODS:`, `VORTEX_NOT_READY:`, `SIGNED_OUT:`, `NO_PATCH:`, `NO_PATCH_FILES:`) match `ui/app.js` today, but nothing enforces it.

**How to apply:** before renaming a command, event or error prefix, change `ui/app.js` in the same piece of work; consider a contract test.
