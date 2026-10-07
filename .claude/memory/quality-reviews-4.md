---
name: quality-reviews-4
description: Open findings from the launcher Tauri shell review (src-tauri/) and the UI/command contract; last of ten reviews
type: reference
verified: 2026-10-07
refs: src-tauri/src/main.rs, src-tauri/src/mods.rs, ui/app.js, .github/workflows/build.yml
---
Read-only review on 2026-10-07 (container lacked GTK/WebKit, so src-tauri's 4 unit tests were not run); no code changed. Verdict: solid and careful, but untested glue and lots of dead code. Report: https://claude.ai/artifact/GePGALbMCxBvW2zNBVxxCG

- CI runs only `cargo test -p launcher-core`, so src-tauri tests, clippy and fmt never run.
- ~780 lines of `mods.rs` (the "Download all mods" queue) are unreachable: `download_all_mods` is not registered. 9 registered commands (Nexus sign-in, `cancel_mods`, ...) are never called by the UI.
- Play waits on the staff report upload and can stall up to ~100 s when rate-limited (`main.rs`).
- Config is written non-atomically and a damaged one silently resets to defaults (`main.rs`).
- The registered clipboard Nexus sign-in sends any key-shaped clipboard text to Nexus (`mods.rs`).
- UI contract: all 35 commands, 7 events and the error prefixes (`NEEDS_NEXUS_MODS:`, `VORTEX_NOT_READY:`, `SIGNED_OUT:`, `NO_PATCH:`, `NO_PATCH_FILES:`) match `ui/app.js` today, but nothing enforces it.

**How to apply:** before renaming a command, event or error prefix, change `ui/app.js` in the same piece of work; consider a contract test.
