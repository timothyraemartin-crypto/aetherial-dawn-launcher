---
name: launcher-parts
description: What ui/, src-tauri/, core/ and vortex-extension/ each do, and what Play runs
type: reference
verified: 2026-10-07
refs: ui, src-tauri/src/main.rs, core/src/lib.rs, vortex-extension/index.js
---
Windows app that puts a player's Skyrim into the state the server needs, signs them in with Discord, then starts the game.
- `ui/`: plain JS screens (Home, Server, Mods, News). No file or network work; only calls Tauri commands.
- `src-tauri/`: Windows shell, ~30 commands, the Play sequence in `main.rs`, "Download all mods", staff server-mods export, face sharing, menu music, self-update.
- `core/`: the testable Rust library, grouped by job: game build detection/fixing (version checks, patching, MulderLoad community patches, Steam downgrade fallback); SkyMP file sync; Discord sign-in, health checks, crash reports; mod list and installers; game-folder tidying; matching the server's plugin order; the Vortex bridge.
- `vortex-extension/`: read-only helper that answers only launcher-signed requests on localhost.
- Play: install SKSE, tidy mods, set load order, health checks, get a Discord game session, start the game through SKSE.
- Fetched from the server: client manifest, `mods.json`, `masters.json`, patches, status, login service.

**Why:** the README says Discord sign-in and the downgrader were tested only against stand-ins, not the live services.

**How to apply:** put logic in `core/` (unit-testable), keep `ui/` and `src-tauri/` thin. Full page: https://claude.ai/artifact/TKnhzamYtqMX9e1CXJhLyg
