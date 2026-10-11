# Our own launcher-delivered mods that would ease running the server

These are proposals from what the server notes show; none is built. "Server" means the SkyMP gamemode modules already deployed through QA.

1. **Staff overlay (F3 menu staff page)** - on-screen list of players, one-click kick/ban/teleport/spectate for staff tiers. Cuts SSH/Discord trips. Mostly exists in staffops/F3; recommend finishing it as a client panel. RECOMMENDED first.
2. **One-click bug/crash report** - key press attaches last log, position, nearby players and sends to the Discord bridge. Crash-reports feature exists server-side; client button is the missing half. RECOMMENDED second.
3. **Server status HUD** - tiny corner readout: ping, players online, server version, "restart in N min" warning so nobody gets surprised by a restart. Low effort.
4. **Never-pause / focus keeper** - just Skyrim Souls RE + bAlwaysActive (see FINDINGS.md); already required, needs only the gate fix. Do this immediately.
5. **Self-diagnostics bundle** - launcher "Fix it" that checks load order vs server masters and mismatches (stops "I can't join" tickets).

Recommendation: do 4 now, then 2, then 1. Skip 3 and 5 until tickets show a need.
