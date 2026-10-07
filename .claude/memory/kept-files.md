---
name: kept-files
description: Odd-looking launcher files that are intentionally kept, and what the dead-file cleanup removed
type: gotcha
verified: 2026-10-07
refs: core/src/bin/lane-hash.rs, core/testdata/bzip2.zip, core/testdata/lzma.zip, docs/examples/racemenu-sync-guard.json, src-tauri/icons/128x128@2x.png, docs/masters-json.md
---
Keep these even though nothing names them: `core/src/bin/lane-hash.rs` (Cargo builds it; dev tool the Mods chat uses for list hashes); `core/testdata/*.zip` and `docs/examples/racemenu-sync-guard.json` (loaded by Rust unit tests via `include_bytes!`/`include_str!`); `src-tauri/icons/128x128@2x.png` (Tauri bundler finds icons by convention); `docs/masters-json.md` (documents the masters-json tool).
Removed in launcher PR #51 (nothing referenced them): `ui/art/background-dawn.jpg`, `ui/art/icon-128.png`, `docs/connect-vortex-runbook.md`.

**Why:** a name search finds no reference for these, so a naive dead-file sweep would break tests or the bundle.

**How to apply:** before deleting, check `include_bytes!`/`include_str!`, Cargo bin auto-discovery and Tauri icon conventions, not only grep for the name.
