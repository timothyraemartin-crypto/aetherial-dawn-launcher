# NoPause: unpausing every menu (plan and status)

Goal (Timothy, 2026-10-11): the game world never pauses behind a menu, including Esc, Tab, Skills and Map.

## What 1.0.8 does
- Hooks `UIMessageQueue::ProcessCommands` (Address Library id 82167, 1.6.1170) and, after each frame's menu messages, looks at the open menu stack.
- For each allow-listed menu it removes the pause bit and `kFreezeFrameBackground` and subtracts exactly the one pause count the engine added for that menu (tracked per menu object, never below 0). Nothing else writes `UI::numPausesGame`.
- Why `kFreezeFrameBackground` goes too: an unpaused menu that keeps it deadlocks the game (the likely cause of the earlier Esc hang). Idea from reading the open-source Skyrim Souls RE; no code copied.
- Defaults: unpaused = Journal (Esc), Tween (Tab), Map, Inventory, Magic, Favorites, Container, Crafting, Barter, Gift, SleepWait, Training. Still paused = Stats (Skills), Book, LevelUp, MessageBox, Lockpicking, Tutorial, Console.
- `AetherialDawn-NoPause.ini` can set any menu to 0/1 (`[Menus] Tween=0`), `[Options] AllowFreezeFrame=0` (leave freeze-frame menus paused again) and `Heartbeat=0`. Deleting the DLL turns the mod off.
- Log: `My Games\Skyrim Special Edition\SKSE\AetherialDawn-NoPause.log` (previous run kept as `.prev.log`), one `hb ...` line per second while a known menu is open, so a hang shows as the last heartbeat.

## Still open
- Skills (Stats): unpausing gave a black screen, no controls and blur. The reference mod patches the Stats menu's own code (freeze flag at show, a controls check, two muted sounds). That needs verified code offsets for 1.6.1170. Plan: log the bytes at those spots first, patch only when they match. Until then Skills stays paused.
- Book: re-enable after the first in-game session is clean.

## Staged tests (launcher builds, no hand-editing)
1. Esc, Tab, Map unpaused (this build): report blur, lost controls, freezes; read the heartbeat.
2. Book back on.
3. Skills with the Stats patches.
