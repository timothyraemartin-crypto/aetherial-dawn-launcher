---
name: quality-reviews
description: Open findings from the launcher quality reviews (sign-in and health, plugin order); more reviews pending
type: reference
verified: 2026-10-07
refs: core/src/auth.rs, core/src/health.rs, core/src/serverorder.rs, core/src/serverlane.rs, core/src/loadorder.rs
---
Read-only reviews on 2026-10-07; no code changed. Vortex has its own topic: topic vortex-extension-review. Unproven items say so.

**Sign-in and health checks** (report: https://claude.ai/artifact/NXj5Tw5yRXcMddHcyNSqm1)
- The launcher calls `/api/users/login-discord/token`, `/api/client-status`, `/api/crash-reports`; these exist on discord only on unmerged branches (e.g. triple-check-fixes), not main. Which branch the live server runs is unknown. See discord memory `launcher-endpoints-unmerged`.
- Master-file check passes when the server gives no size or hash. Loopback listener serves one connection at a time. Network and report paths have thin tests. The `auth-data-no-load.js` name is an untested guess.

**Plugin order matching** (launcher main 4fc172b; 27 serverorder/serverlane tests pass)
1. plugins.txt can be wiped with no backup: `serverorder.rs` reads it with `read_to_string(..).unwrap_or_default()`, a non-UTF-8 (ANSI) plugin name gives empty text, which is then overwritten with only server plugins, and `keep_backup` is skipped for empty text. Same read pattern in `game_order` and several places in `loadorder.rs`. Inferred from code, not reproduced.
2. `health.rs` reports OK on an unusable masters list.
3. No pinned `list_hash` test in `serverlane.rs`, so a `ModEntry` change silently changes every list hash.
4. `masters_json` does not enforce the 254-plugin MAX_FULL limit.
Minor: byte-at-a-time crc32 is slow.

**How to apply:** fix 1 first (data loss); read bytes and decode lossily, and back up before any overwrite.
