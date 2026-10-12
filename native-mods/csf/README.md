# Custom Skills Framework route for the Tab skills menu (draft backup)

Why: our own `AetherialSkillTree.dll` rewrites the vanilla Tab menu in memory. If 0.1.4 does not work in game, this
route lets an established open-source framework draw the menu instead.

- Framework: Exit-9B/CustomSkills (v3, by Parapets, from meh321's original). **MIT** (CONFIRMED from the repo's LICENSE,
  copied here as `LICENSE-CustomSkills.txt`; the notice must ship with any copy). Pinned commit `8be7055f2483a261e0c377e2e8fc04b34116c68f`
  (2026-09-03). Address Library plugin (`UsesAddressLibrary`); 1.6.1170 support is REPORTED (Nexus page), not checked.
- Build: Actions > "Build Custom Skills Framework" (manual) makes `CustomSkills.dll` + sha256 as an artifact. Nothing ships from it yet.
- `SKILLS.json` is generated, not hand-edited: in ingame-ui `node skills/tools/csf-data.js --out <dir>`. The file name
  `SKILLS.json` is what makes the framework use it for the vanilla Tab menu (Settings.cpp). It lists our 18 skills,
  192 perk nodes (positions from tier/column; spacing UNKNOWN until seen in game) and the globals/perks that
  ingame-ui writes into `AetherialDawn.esp`. Stable form IDs live in ingame-ui `skills/recipe-plugin/csf-ids.json`.
- Server feed: ingame-ui sets the level/progress globals and the learned perks from the player's server skills
  (property `ad_csf`, switch `csfBridge` in skills-vanilla-map.json, default false). The menu is display-only: the perk-points
  global stays 0, the server owns what is learned.
- To ship (not done): add `CustomSkills.dll` to `src-tauri/bundled/Data/SKSE/Plugins/` and `SKILLS.json` to `.../CustomSkills/`
  in `src-tauri/src/bundled.rs`, write `AetherialSkillTree.off` so our DLL stays idle (two menus cannot both hook the Tab menu),
  then turn on `csfBridge` on the server. The framework log is `Documents\My Games\Skyrim Special Edition\SKSE\CustomSkills.log`.
- Kill switch once shipped: delete `SKILLS.json` (the Tab menu stays vanilla) and `csfBridge:false` on the server.
