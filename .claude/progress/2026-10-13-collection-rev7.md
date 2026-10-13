# 2026-10-13 Nexus collection revision 7 in the launcher rules

Timothy's new list (nzwgrw rev 7, "make sure all the mods are active"). `server/launcher-rules.json` rev .9: collection revision 7, 28 mods, 18 plugins, level warn, no enablePlugins, no level block. Feed-only: no launcher code change, so no new release.

CONFIRMED (Nexus GraphQL + collection.json + the real archives, refresh workflow run on PR 184): ten new mods: Constructible Object Custom Keyword System, Actor Limit Fix, Custom Skills Framework, FNIS, FSMP, JContainers SE, MCM Helper, RaceMenu, SKSE Menu Framework, XPMSSE. File ids unchanged for the 18 old mods. New DLLs: CraftingCategories, ActorLimitFix, CustomSkills, hdtSMP64, JContainers64, MCMHelper, skee64, SKSEMenuFramework (all listed as detection files, so the mod guard leaves them). New plugins: FNIS.esp, RaceMenu.esp, RaceMenuPlugin.esp, XPMSE.esp, MCMHelper.esp (ESL), MoreCraftableEquipment_USSEP.esp. Dropped from the plugin list: MoreCraftableEquipment_AdvCyrus.esp, MoreCraftableEquipment_Fish.esp (no longer required; the guard still leaves them, `Data/MoreCraftableEquipment` is allowed).

INFERRED: the `allow` prefixes for runtime files of FNIS, XPMSSE, RaceMenu, JContainers (JCData), MCM (Data/MCM); loose files outside them would show in the mod guard's list. Not game-verified.

Server side (not done here): the new .esp files need the server load order and masters.json decision, through the coordinator. USSEP spaced name: unchanged (launcher uses the dash copy; `wanted_name` skips it).
