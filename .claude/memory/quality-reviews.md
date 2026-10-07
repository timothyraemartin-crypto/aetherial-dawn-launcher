---
name: quality-reviews
description: Open findings from launcher quality reviews (sign-in, plugin order, UI); more reviews pending
type: reference
verified: 2026-10-07
refs: ui/app.js, core/src/auth.rs, core/src/health.rs, core/src/serverorder.rs, core/src/serverlane.rs, core/src/loadorder.rs
---
Read-only reviews on 2026-10-07; no code changed. Parts 2 and 3 are topics quality-reviews-2 and -3. Vortex has its own topic: topic vortex-extension-review. Unproven items say so.

**Sign-in and health checks** (report: https://claude.ai/artifact/NXj5Tw5yRXcMddHcyNSqm1)
- The launcher calls `/api/users/login-discord/token`, `/api/client-status`, `/api/crash-reports`; these exist on discord only on unmerged branches (e.g. triple-check-fixes), not main; discord draft PR #4 merges that branch. Which branch the live server runs is unknown. See discord memory `launcher-endpoints-unmerged`.
- Master-file check passes when the server gives no size or hash. Loopback listener serves one connection at a time. Network and report paths have thin tests. The `auth-data-no-load.js` name is an untested guess.

**Plugin order matching** (launcher main 4fc172b; 27 serverorder/serverlane tests pass)
1. [FIX IN PR #54, CI green, awaiting review, not merged: plugins.txt read as bytes, failed read is an error, every rewrite backed up (.aetherial-dawn-backup = first list, .aetherial-dawn-previous = list before latest change), atomic writes; real-game Windows test still to do] plugins.txt can be wiped with no backup: `serverorder.rs` reads it with `read_to_string(..).unwrap_or_default()`, a non-UTF-8 (ANSI) plugin name gives empty text, which is then overwritten with only server plugins, and `keep_backup` is skipped for empty text. Same read pattern in `game_order` and several places in `loadorder.rs`. Confirmed by a reproduced test in the game-folder review (quality-reviews-3).
2. `health.rs` reports OK on an unusable masters list.
3. No pinned `list_hash` test in `serverlane.rs`, so a `ModEntry` change silently changes every list hash.
4. `masters_json` does not enforce the 254-plugin MAX_FULL limit.
Minor: byte-at-a-time crc32 is slow.

**Launcher UI** (solid, nothing blocking; report: https://claude.ai/artifact/9J7JvDtDmscCBAW4jXgBPJ)
- CI browser test `ui-fast-play-test.js` passes 153/153 against a fake Tauri back end; `ui/` has no linter or unit tests.
- `app.js`: the progress listener is registered outside try in `update()`/`patchGame()`, so `busy` can stick true; settings toggles do not roll back when `set_prefs` fails; the "launcher is updating" message is unreachable; news cannot be cleared and player counts go stale after a failed fetch.
- Risk: one ~1,260-line closure with ~8 overlapping flags. The UI depends on 35 Tauri commands and error prefixes like `NEEDS_NEXUS_MODS:`; CI fakes them, so a Rust-side rename is not caught.

**How to apply:** fix 1 first (data loss); read bytes and decode lossily, and back up before any overwrite.
