# Custom Skills Framework route for the Tab skills menu (draft backup)

Why: Timothy dropped our own `AetherialSkillTree.dll` (2026-10-12; launcher PR 142 removed it, so Tab is vanilla now). This route lets an
established open-source framework draw our own skills menu instead. Stage 1 (2026-10-14) only answers "does the stock framework open the
Tab menu on 1.6.1170?". Later stage (only if stage 1 works): a small fork that builds the menu live from server data and hides skills the
player did not pick. Plan: /mnt/project-files/aetherial-dawn/mods/custom-skills-framework.md. Nothing in this folder ships.

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
- To ship (not done; HELD until launcher 0.1.154 is released and Timothy has played once without a black screen; also needs the guard allowlist entry in launcher-rules.json and the signed feed, no publish before that): add `CustomSkills.dll` to `src-tauri/bundled/Data/SKSE/Plugins/` and `SKILLS.json` to `.../CustomSkills/`
  in `src-tauri/src/bundled.rs`, then turn on `csfBridge` on the server. The framework log is `Documents\My Games\Skyrim Special Edition\SKSE\CustomSkills.log`.
- Kill switch once shipped: delete `SKILLS.json` (the Tab menu stays vanilla) and `csfBridge:false` on the server.

## Status 2026-10-14 (after the 00:39Z coordinator update)
- The framework DLL is NOT ours to ship for stage 1: `server/launcher-rules.json` already lists "Custom Skills Framework" 3.2.0 (Nexus mod 41780, file 799487) from the collection, with `Data/SKSE/Plugins/CustomSkills` allowed, so the SKSE guard allowlist entry exists. REPORTED (coordinator): CustomSkills.dll already loads on Timothy's PC (skse64.log 00:30Z). Bundling a second copy built here could fight the collection's file; `build-csf.yml` stays only as a fallback/fork starting point.
- `build-csf.yml` cannot be started from this branch: the dispatch API answers 404 until the workflow file is on main.
- `SKILLS.json` must NOT be bundled yet: it names forms in AetherialDawn.esp (globals/perks 0x05001000+) that do not exist until the plugin is regenerated with csf records (held with masters.json/parity). With the file but without the forms, the framework would replace the vanilla Tab menu with nodes that point at missing records. Ship the file in the same step as the regenerated plugin (add it to `FILES` in bundled.rs, path `Data/SKSE/Plugins/CustomSkills/SKILLS.json`, ideally behind a launcher-rules switch so it can be turned off without a release).
