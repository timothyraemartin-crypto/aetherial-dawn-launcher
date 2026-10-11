# Never-pause findings (2026-10-10)

## 1. What was built
CONFIRMED: no custom never-pause plugin exists. No C++/CMake project anywhere under A:\Aetherial Dawn, the launcher source or ClaudeWork.
CONFIRMED: the job was done by Skyrim Souls RE v2.4.0, a required mod in the launcher catalog. Its log (My Games\...\SKSE\SkyrimSoulsRE.log, 2026-10-10 19:18) says "Hooks installed", "Menu hooks installed". Default ini unpauses every menu (inventory, magic, map, journal, tween, barter, container, lockpicking, console, etc.) and lets you move in menus.
CONFIRMED: it is gone from the game now. Set-aside pass 2026-10-10 23:40Z moved SkyrimSoulsRE.dll/.ini/.pdb (and CrashLogger.dll) to A:\Aetherial Dawn\backups\mods-set-aside\2026-10-10T23-40-25-333Z\. So a "no mods" Play right now pauses in every menu.
A bespoke SKSE C++ plugin is NOT needed; building one would only duplicate SSRE.

## 2. Why the game still pauses
- Menus (inventory, journal/Esc, map, magic...): vanilla pauses. Fixed only while SSRE is installed. Dialogue/crafting already never pause. Main menu, load screens, loading, and the Esc "system" page inside Journal follow SSRE's bJournalMenu (true in the saved ini). INFERRED, needs a game check.
- Window losing focus (alt-tab, click other monitor): vanilla Skyrim freezes. CONFIRMED: Skyrim.ini [General] has no bAlwaysActive, SkyrimPrefs.ini has none, and neither SSRE nor SkyrimPlatform.ini sets it. With the server's 20 s drop timeout (patched from 60 s) a frozen client gets removed. INFERRED fix: bAlwaysActive=1 under [General] (well known for Skyrim, a Starfield forum report agrees; not verified on this PC).
- Skyrim.ini is derived from Skyrim.ini.base / .baked beside it, and the launcher's gameini.rs rewrites it (archive lists). Put bAlwaysActive in the launcher's ini step, not by hand-editing.

## 3. Fix package (staged, NOT installed)
package\Data\SKSE\Plugins\SkyrimSoulsRE.dll (sha256 04bb6686...f88bbf) + .ini (a4840dea...25bf), byte copies from the set-aside backup. Needs Address Library bin for 1.6.1170 (present in game Plugins) and SKSE 2.2.6 (present). Engine Fixes is recommended by SSRE (log warning) but not required.

## 4. What the coordinator / Launcher thread must do
1. No-mods Play gate must allow SkyrimSoulsRE.dll + SkyrimSoulsRE.ini (and ideally CrashLogger.dll, which the code comment already calls "never set aside"). The set-aside list should be an allow-list: SKSE, versionlib bins, SkyrimPlatform, MpClientPlugin, SSRE, CrashLogger.
2. Launcher ini step: ensure `bAlwaysActive=1` in [General] of Skyrim.ini (and its .base). I won't edit launcher source.
3. Game restart for a test (coordinator): restore the two SSRE files, add bAlwaysActive=1, launch, then check: open inventory/map/Esc with another player nearby moving; alt-tab 60 s and confirm no disconnect. I have not run the game.

## 5. Book menu (2026-10-11)
REPORTED (Timothy): a player opening a book breaks the game. Not reproduced here (no game access); freeze, crash or stuck menu is UNKNOWN.
INFERRED: Book was the only reading/item-view menu on the allow-list that the engine ran paused with its own 3D book scene; NoPause 1.0.5 cleared its pause flag and decremented numPausesGame, so the world kept running under it. Code paths show no crash route (Readable + SEH guards), so the likely symptom is a stuck or misbehaving book menu, not a hard crash.
FIX: bundled AetherialDawn-NoPause.ini (written once by the launcher) sets Book=0, the DLL's existing per-menu switch, so books stay vanilla-paused. No DLL rebuild. Source default is still Book=1 until a rebuild.
