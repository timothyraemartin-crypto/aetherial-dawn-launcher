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

## D-2026-10-11-3 The launcher is a profile (supersedes the block in D-2026-10-11-1)
Timothy, 2026-10-11: however many mods a player has installed, the game should only run the allowed files.

`core/src/profile.rs`: before the game starts, every extra file found by `modguard::extras` is moved (same-drive rename, instant) into `.aetherial-dawn/aside/<time>/` and moved back when Skyrim closes. The list is written to `aside/journal.json` before the first move, so a crash or power cut is repaired by `profile::restore`, which also runs whenever the launcher starts with no game running and when Play stops before the game starts. A file with something new at its old place is never overwritten; it stays in the aside folder and is logged. A file that cannot be moved (locked) stops Play with `MODS_FOUND:` as before, so the block remains the fallback. Rejected: a separate clean folder of hard links (Steam and SKSE expect the real folder; more unknowns) and a plugins.txt-only profile (does not stop loose files, archives or SKSE DLLs).

## D-2026-10-11-4 Never pause: bAlwaysActive, no Skyrim Souls RE
Timothy, 2026-10-11: "No I dont want the mod I want to create our own of it." Skyrim Souls RE stays out of the allowed set. On every Play the launcher writes `bAlwaysActive=1` under `[General]` in Skyrim.ini and Skyrim.ini.base (original kept as `.aetherial-dawn-backup`), so the game keeps running when its window loses focus. Our own never-pause plugin comes from the Never-pause thread; its file name is added to the allowed set when it exists.

## D-2026-10-13-1 NoPause removed, third-party unpause mod instead
Timothy, 2026-10-13 00:56Z: "im going to have all the no pause stuff removed and adding a mod to the server that will require it". Supersedes D-2026-10-11-4 and the own-mods-only rule for this one mod. AetherialDawn-NoPause is retired (bundled.rs RETIRED). Mod choice: whatever his Nexus collection contains (Timothy 2026-10-13 01:01Z: launcher checks players against the collection's mod list and load order).
