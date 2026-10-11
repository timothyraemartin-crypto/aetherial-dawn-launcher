# Skill tree mod: case list (Timothy's rule: cover all the bases, not one small area)
Default for any case not yet reviewed: leave the vanilla behaviour untouched. Kill switch: Data\SKSE\Plugins\AetherialSkillTree.off. Log: Documents\My Games\Skyrim Special Edition\SKSE\AetherialSkillTree.log.

| # | Case | Status |
|---|------|--------|
| 1 | Each of our 18 skills shows with right name, level, XP, perks (one by one) | built in 0.1.0 (names, perks, descriptions); levels via Lab AV sync; NOT game-tested |
| 2 | Only learned skills visible; none learned; skills reset by staff | built (list filter; none learned -> list untouched); NOT game-tested |
| 3 | Vanilla skills (the 18 vanilla constellations) must not leak back into the menu | not started |
| 4 | Open Stats, close Stats, repeat quickly, open from different places (Tab, pause menu) | probe only |
| 5 | Tween menu open/close, switching Tween -> Stats -> Tween | not started |
| 6 | Multiplayer client: server not connected, connecting, reconnect, relog, no data yet | not started |
| 7 | NoPause (1.0.2 keeps Stats and Tween paused) and FreezeMenus interaction | not started |
| 8 | Perk learned/refunded while the menu is open; XP gained while open | not started |
| 9 | Gamepad, window focus loss, level-up/skill-increase messages, saves/loads | not started |
| 10 | Plugin disabled (.off) or data missing: vanilla menu exactly as before | built (no state -> nothing applied; names restored on hide); NOT game-tested |
| 11 | No per-tick inventory/equipment calls; no Papyrus; menu data read only on open | by design |
