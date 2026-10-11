# Skill tree UI plan (for Timothy's OK; nothing below is built yet)

## What the player sees
- Tab opens Skyrim's own constellation menu (same camera, animation, star layout). Only the skills the player has learned (3 today) are in the menu; their constellations carry our names, our perk names and descriptions, and our ranks/levels/XP/learned perks from the server.
- Look: Aetherial Dawn theme (navy #05080C, ice blue #B6DCF7/#5EA4D8, gold #D8B56A, text #ECE6D6) applied as loose texture replacements for the sky and star glow, tinted from launcher-art (background-dawn.jpg palette, emblem.png). Text uses the game's own Skyrim font (a custom font in the 3D menu is not possible without editing the SWF).

## Which constellation shows which of our skills (proposal, fixed table)
Matching by the real star counts read from the game (vanilla perk counts incl. root: One-Handed 12, Two-Handed 12, Archery 11, Block 11, Smithing 12, Heavy 9, Light 8, Pickpocket 9, Lockpicking 12, Sneak 12, Alchemy 12, Speech 10, Alteration 18, Conjuration 23, Destruction 15, Illusion 15, Restoration 13, Enchanting 12). Our trees need 7 to 12 perks.
- Combat: One-Handed->One-Handed, Two-Handed->Two-Handed, Archery->Archery, Block->Block, Heavy Armour->Heavy Armor, Light Armour->Light Armor.
- Crafting: Blacksmithing->Smithing, Alchemy->Alchemy, Cooking and Brewing->Enchanting, Leatherworking->Lockpicking, Tailoring->Sneak, Jewelcrafting->Pickpocket.
- Gathering: Mining->Destruction, Woodcutting->Speech, Harvesting->Illusion, Hunting->Restoration, Fishing->Alteration, Farming->Conjuration.
Final table is chosen by star count once the code reads each constellation's real node graph; the table lives in a data file so it can change without a new DLL.

## "Hide" what a skill the player does not have
- A skill not learned is removed from the menu's skill list and from the camera's selectable list, so you can only look at yours. If the game still draws its baked stars in the sky, they stay dim and unnamed (this part is tested in game first; I can't promise the sky itself changes).
- No data yet (not logged in, no skills chosen, server not connected): the menu stays exactly vanilla. Unknown = untouched.

## Limits
- Constellation SHAPES are baked into the game's meshes, so star positions stay vanilla. A skill with fewer perks than the stars of its constellation leaves some stars unused (shown as locked, no name). I will not move stars.
- Learning a perk still happens in F3 for now.
- Levels, XP and learned perks come from the Lab's ad_vanilla property. It needs a small extension (a per-skill block for all 18 skills); the Lab thread owns that, so I will ask through the coordinator. No new property from me.

## Cases (all in CASES.md) and safety
Kill switches: AetherialSkillTree.off (idle), AetherialSkillTree.norename (log only). Log readable while the game runs. Menu data is touched only when the menu opens, no per-tick inventory/equipment calls. Ships through the launcher bundle with an in-game checklist.
