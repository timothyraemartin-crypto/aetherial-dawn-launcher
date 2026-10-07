---
name: quality-reviews-2
description: Open findings from launcher quality reviews, part 2 (SkyMP file sync first); continues quality-reviews
type: reference
verified: 2026-10-07
refs: core/src/manifest.rs, core/src/sync.rs, core/src/settings.rs
---
Read-only reviews on 2026-10-07; no code changed. Part 1 is topic quality-reviews. Put new reviews here until ~2.5 KB, then start part 3.

**SkyMP file sync** (grade B; manifest.rs, sync.rs, settings.rs; 9/9 tests pass; report: https://claude.ai/artifact/BCS1BHhWmFwCThdwKkr4Qc)
- Solid: hash-verified files, temp-then-rename writes, path-escape checks.
- High: the manifest is unsigned and hashes come from the same host as the files (the repo already has an ed25519 key for signed scripts that could be reused). `remove` can delete any file in the game folder, and a path listed in both files and remove is downloaded then deleted every run. The first failed download aborts with no retry, leaving a mix of old and new files.
- Medium: no download size cap; one locked file aborts the check; path checks miss Windows reserved names and case; an unparseable settings file silently drops the player's other keys.
- Untested: `apply`, `download_verified`, `replace`.

**How to apply:** treat the signed-manifest and `remove` scoping fixes as prerequisites before widening what the server can push.
