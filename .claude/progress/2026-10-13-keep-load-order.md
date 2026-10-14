# Keep the server's plugins.txt during the first 10 minutes of a session (draft)

Why: bot journal 23:42 and 23:44Z (REPORTED via coordinator): Timothy's parity reading MATCHed at launch, then ~45 s later DRIFTed with 4 Creation Club plugins and the spaced USSEP switched on ahead of the server's plugins (crash card CR-0155); black screen. plugins.txt is correct when Play writes it and is rewritten by something else after launch. Writer UNKNOWN (Vortex re-deploy, Steam Creation Club or the game; INFERRED Vortex).

Change: `keep_load_order` (src-tauri/src/main.rs) runs after the game starts: every second for 600 s it first points a switched-on spaced original (USSEP) back at the launcher's dash-named link (aliases::point_at_links) and then runs `serverorder::set_exact`; when that changes the file it logs when, and which plugins were on, so the writer's timing is on record. Kill: rules `checks.keepLoadOrder:false`, status.json `launcher.keepLoadOrder:false`, or env `AD_KEEP_ORDER_OFF=1`.

Limits: core tests pass (230); src-tauri does not build on Linux, so the Windows build is unchecked. Whether the game already read the list before a rewrite is UNKNOWN, so this may not stop a black screen. NEEDS_GAME_CHECK on Timothy's PC (0.1.143 rule) before any release.

Timothy 23:56Z: wants the spaced USSEP allowed ticked in Vortex. Master names inside plugins that need USSEP are already rewritten by aliases.rs (rename_masters); the dash-named link is what the game runs. This change makes a ticked spaced original in plugins.txt a ticked link (same `*`, same place) instead of just off, so USSEP stays loaded and the file stays consistent. Implemented, PC-unverified; effectiveness UNKNOWN.
