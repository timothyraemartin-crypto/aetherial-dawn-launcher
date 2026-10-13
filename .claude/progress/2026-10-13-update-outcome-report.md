# 2026-10-13 Update button reports its outcome (listeners, for 0.1.151)

Status: implemented, DRAFT. Not compiled here (src-tauri needs the GTK/Windows toolchain this container lacks: UNKNOWN until the Windows build job), not merged, not released, not game-verified.

What: after the Update button finishes (fetch, plan, apply), the launcher posts `{event:"update", result:"ok"|"error", launcher}` to `{AUTH_URL}/api/client-event` in the background (new `send_client_event` in `src-tauri/src/main.rs`). Event and result only: no names, paths or accounts. 404 and any failure are silent for the player. "Another operation is running" and "no game folder yet" are not counted as failures. The bot counts `bot.launcher.update.ok|error` on the Game State scoreboard.
Depends on: discord PR "POST /api/client-event" (same branch name) merging and the bot restarting first; until then the route answers 404 and the launcher ignores it. The `client-event` contract entry is added to `contract/launcher-server.json` here and in the discord repo; the launcher copy of the file currently lacks entries the discord copy gained (characters-mine, portrait-self, portrait-file), so contract-in-sync will differ until those land here.
Checks run: `node .github/scripts/contract-check.js` (12 endpoints, 12 call sites), `cargo test -p launcher-core --test server_contract` (4 pass). `cargo check` of src-tauri failed on missing gdk-3.0, not on the code.
Rides 0.1.151 when the launcher thread cuts it. Install, collection and sync buttons can call the same function later.
