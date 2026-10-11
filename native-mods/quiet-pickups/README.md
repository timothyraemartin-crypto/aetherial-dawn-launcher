# AetherialQuietPickups 1.0 (BUILT, NOT INSTALLED, NOT GAME-TESTED)
Hides vanilla top-left "X added/removed" HUD notices. One MinHook detour on HUDMenu::ProcessMessage (vtable slot 4,
AE ids 215362 / 216313, 1.6.1170 only); event-driven, no per-tick or inventory/equipment calls; SEH-guarded.
Build: skse-toolchain\build.bat src\main.cpp AetherialQuietPickups.dll (adlite kit, no CommonLib).
Install (later, via launcher): Data\SKSE\Plugins\AetherialQuietPickups.dll + .ini (needs versionlib bin + SKSE 2.2.6).
Launcher no-mods gate file names: AetherialQuietPickups.dll, AetherialQuietPickups.ini.
ini: Enabled, LogNotifications (1 = log every HUD notice + HIDDEN/shown to My Games\...\SKSE\AetherialQuietPickups.log),
HideContains (case-insensitive substrings, default " added; removed").
Game test: pick up an item; the log must show the notice -> HIDDEN. If pickups never appear in the log, vanilla uses another
path (UNKNOWN until run). If the vanilla text isn't "... added", adjust HideContains from the log.
sha256 dll: BB3C03442DC1C34EEC2A7240ACEE5B642C96DE0BE4D3AF1BCED9A3D70127814F
