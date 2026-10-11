# Decisions (newest first)

## D-2026-10-11-1 No mods: Play refuses while anything extra is installed
Timothy, 2026-10-11: the launcher checks the game version on every launch and must not let a player join if any mod beyond the allowed set is installed; the allowed set is exactly none.

Implemented in `core/src/modguard.rs`, called from `play_run` before the launcher changes anything on the PC. Play stops with `MODS_FOUND:` and the page lists what to remove; pressing Play checks again. The game version check already ran at the start of every Play (`auto_version`).

Allowed (everything else counts as a mod):
- Plugins: the server's base masters from `masters.json` (5 today), `_ResourcePack.esl`, and the official Creation Club files. Creation Club is Bethesda's own content that Steam installs and `Skyrim.ccc` lists; it is not a player mod, and the launcher already switches it off for the session (`hide_ccc`, so the server's plugin order matches). Names are matched by `Skyrim.ccc` and by the official `cc<author>sse<number>-` / `_` pattern, so a crash that left `Skyrim.ccc` empty does not lock a player out.
- Archives: the game's own (`Skyrim - *.bsa`, `Dawnguard/Dragonborn/HearthFires/Update.bsa`, `MarketplaceTextures.bsa`, `_ResourcePack.bsa`) and Creation Club's.
- SKSE plugins: SkyMP's `MpClientPlugin.dll` and `SkyrimPlatform.dll` (and anything the client manifest lists). SKSE itself sits in the game folder, not in `Data`, and Address Library is `.bin` data, so neither counts.
- Loose files under `Data`: only those in `client/manifest.json`. `Data/Platform` (SkyMP), `Data/Scripts` (SKSE and SkyMP Papyrus files), `Data/Video` and `Data/ShaderCache` are not looked at. Vortex's empty `__folder_managed_by_*` markers are ignored.

Limits: this only stops the new launcher from starting the game. The server cannot see a player's mods, so an old launcher or a hand-started game is not blocked by it; enforcing that needs a server-side minimum launcher version (the `launcher.minVersion` notice is advisory today).

## D-2026-10-11-2 A windowless Skyrim is ended automatically
Timothy, 2026-10-11: force-closing Skyrim left it running, so Play refused. `watch::end_when_window_closed` now ends it 4 s after its window goes (was 15 s); `end_leftover_skyrim` ends a SkyrimSE.exe that is at least 30 s old and has had no window for 8 s, when this launcher has no session of its own (a leftover from an earlier launcher).
