# 2026-10-13 Havok physics step follows the player's frame cap

Why: creatures sink over rocks on the hosting player's game (creature desync thread). Community guidance (STEP, REPORTED): cap fps at refresh-1 via SSE Display Tweaks `FramerateLimit` and set Skyrim.ini `[HAVOK] fMaxTime = 1 / cap`. Whether this fixes the sinking is UNKNOWN.

Changed (implemented, core tests pass 228/228):
- `gameini::TUNABLE` gains `Skyrim.ini [HAVOK] fMaxTime` (0.004 to 0.034).
- New tune mode `framerate` (only for fMaxTime): on Play the launcher reads `Data/SKSE/Plugins/SSEDisplayTweaks.ini` `[Render] FramerateLimit` (30 to 240), writes 1/limit (7 decimals), backs up the ini once. No usable cap = nothing written.
- `server/launcher-rules.json`: entry added with `on:false` (the shipped-file test requires today's behaviour). Enable = flip to true after a launcher with this mode is released.

NOT verified: src-tauri compile (needs the Windows job), any in-game effect. Timothy's PC: 120 Hz, so cap 119 and fMaxTime 0.0084034; his FramerateLimit must be set in SSEDisplayTweaks.ini or nothing is written.
Kill switch: rules `on:false`, or remove the entry. Next: Windows build, release, flip the rule, compare `[ad-movesync] dip:` counts.

## Update 22:40Z: frame cap for everyone (Timothy: cap at 60 fps)
- `SSEDisplayTweaks.ini [Render] FramerateLimit` (30 to 240) is now a tunable key; Play applies it to `Data/SKSE/Plugins/SSEDisplayTweaks.ini` before reading the cap, so fMaxTime follows (60 gives 0.0166667). If that ini does not exist yet nothing is written that Play; the next Play (after the game has made it) does.
- Rules entry `FramerateLimit set 60` ships `on:false`; 0.1.152 is not tagged yet, so this rides 0.1.152.
- A 60 cap on a 120 Hz monitor is a visible change. Off switch: rules `on:false` for both entries (values already written stay; the first original is kept as `<name>.aetherial-dawn-backup` next to the ini).
