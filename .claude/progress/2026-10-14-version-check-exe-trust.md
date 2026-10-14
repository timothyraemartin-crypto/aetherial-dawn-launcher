# 2026-10-14 False "Steam has updated your game data" after a crash

Timothy's 0.1.154 (screenshot + 00:30Z session report): Server Info shows "Skyrim 1.6.1170 (!) Steam has updated your game data past 1.6.1170" while SkyrimSE.exe is 1.6.1170.0 and the health check passes Skyrim version and "All 11 match the server". Fix version then fails NO_PATCH "already the server's version" (patch list /launcher/patches/index.json is 404).

CONFIRMED from code (core/src/version.rs `check`): with the exe at the target, the check compares Steam's appmanifest depot manifests with the server's; when they differ and no version record holds, it says needed. Steam's record never changes after a hand downgrade, so only the hand-set "already on this version" mark keeps it quiet. `forget_manual` (src-tauri main.rs, after any crash) deleted that mark, so any crash (here 0xC0000409 after 3 s, not a version problem) brought the warning back.

Change: (1) `version::forget_manual_after_crash` keeps the mark while SkyrimSE.exe is the server's build (kill switch env AD_FORGET_MANUAL_ALWAYS=1 = old behaviour). (2) ui/app.js: when patch_game answers "already the server's version", run `mark_game_ok` (what "My game is already on this version" does) instead of showing a dead end. The check itself is unchanged, so a Steam-rewritten data folder is still caught by the record.

Offline-tested: new unit test in version.rs (11 version tests pass). main.rs and the UI path run only in Windows CI / on a PC. Not game-verified. NEEDS_GAME_CHECK: after the update, press Fix version (or Play) once; the warning clears and survives a crash.
