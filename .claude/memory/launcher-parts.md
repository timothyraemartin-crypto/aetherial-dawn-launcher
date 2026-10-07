---
name: launcher-parts
description: What ui/, src-tauri/, core/ and vortex-extension/ each do, and what Play runs
type: reference
verified: 2026-10-07
refs: ui, src-tauri/src/main.rs, core/src/lib.rs, vortex-extension/index.js
---
Windows app that puts a player's Skyrim into the state the server needs, signs them in with Discord, then starts the game.
- `ui/`: plain JS screens (Home, Server, Mods, News). No file or network work; only calls Tauri commands.
- `src-tauri/`: Windows shell, ~30 commands (the unreachable "Download all mods" queue is removed in draft PR #57, see quality-reviews-4), the Play sequence in `main.rs`, staff server-mods export, face sharing, menu music, self-update.
- `core/`: the testable Rust library, grouped by job: game build detection/fixing (version checks, patching, MulderLoad community patches, Steam downgrade fallback); SkyMP file sync; Discord sign-in, health checks, crash reports; mod list and installers; game-folder tidying; matching the server's plugin order; the Vortex bridge.
- `vortex-extension/`: read-only helper that answers only launcher-signed requests on localhost.
- Play: install SKSE, tidy mods, set load order, health checks, get a Discord game session, start the game through SKSE.
- Fetched from the server: client manifest, `mods.json`, `masters.json`, patches, status, login service.

**Why:** the README says Discord sign-in and the downgrader were tested only against stand-ins, not the live services. The Steam downgrader (`core/src/downgrade.rs`) is not called by the app; the shipped route is the MulderLoad patch, so that README line is outdated.

**How to apply:** put logic in `core/` (unit-testable), keep `ui/` and `src-tauri/` thin. Full page: https://claude.ai/artifact/TKnhzamYtqMX9e1CXJhLyg
