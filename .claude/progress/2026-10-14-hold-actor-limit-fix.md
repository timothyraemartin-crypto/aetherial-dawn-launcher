# 2026-10-14 Hold back Actor Limit Fix (rules `hold`)

Timothy's 0.1.154 session 00:30:52Z: Skyrim crashed 3 s after start (0xC0000409); skse64.log ends at "loading plugin Actor Limit Fix" (CONFIRMED by the coordinator from the log), and a box "ActorLimitFix.dll AddressLibrary.cpp(102,34): Identifier not found, 523948" (REPORTED screenshot). Address Library 13 is installed and the health check passes. Why ID 523948 is missing from the 1.6.1170 file: UNKNOWN.

Change: `CollMod.hold` (core/src/collection.rs). `Collection::owns` no longer exempts a held mod, so the existing no-mods guard (main.rs, unchanged) sets its files aside for the session and puts them back at close; the collection row lists it as "Set aside by the launcher for now", not Missing. server/launcher-rules.json: Actor Limit Fix `hold: true`, rev 2026-10-14.1 (publish needs the feed workflow; old launchers ignore the field). Kill switch: republish without `hold`.

Checks: cargo test -p launcher-core 235 pass (1 new). No src-tauri change. Not merged, released, published or game-verified.

Not held: JContainers64.dll, which SKSE reports as "disabled, incompatible with current version of the game" (a log line, not a crash; anything needing it fails: REPORTED). The rev 7 order warnings (RaceMenu after FNIS, MCMHelper after MoreCraftableEquipment_USSEP and SkyUI) are warn-only in the health row and do not stop Play; they are not the cause of this crash and not shown to cause the earlier black screen (that was the spaced USSEP name and Creation Club plugins on, INFERRED). The "MoreCraftableEquipment_USSEP-AD master not switched on" line is UNKNOWN until the log line names the master.
