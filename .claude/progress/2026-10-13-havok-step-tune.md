# 2026-10-13 Havok physics step follows the player's frame cap

Why: creatures sink over rocks on the hosting player's game (creature desync thread). Community guidance (STEP, REPORTED): cap fps at refresh-1 via SSE Display Tweaks `FramerateLimit` and set Skyrim.ini `[HAVOK] fMaxTime = 1 / cap`. Whether this fixes the sinking is UNKNOWN.

Changed (implemented, core tests pass 228/228):
- `gameini::TUNABLE` gains `Skyrim.ini [HAVOK] fMaxTime` (0.004 to 0.034).
- New tune mode `framerate` (only for fMaxTime): on Play the launcher reads `Data/SKSE/Plugins/SSEDisplayTweaks.ini` `[Render] FramerateLimit` (30 to 240), writes 1/limit (7 decimals), backs up the ini once. No usable cap = nothing written.
- `server/launcher-rules.json`: entry added with `on:false` (the shipped-file test requires today's behaviour). Enable = flip to true after a launcher with this mode is released.

NOT verified: src-tauri compile (needs the Windows job), any in-game effect. Timothy's PC: 120 Hz, so cap 119 and fMaxTime 0.0084034; his FramerateLimit must be set in SSEDisplayTweaks.ini or nothing is written.
Kill switch: rules `on:false`, or remove the entry. Next: Windows build, release, flip the rule, compare `[ad-movesync] dip:` counts.
