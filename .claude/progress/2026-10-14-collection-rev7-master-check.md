# 2026-10-14 Collection rev 7: master check in the Play health row

Draft PR claude/collection-rev7-1eybvu (Timothy 23:55Z: "Just put the new collection"). Adds `master_problems` to `core/src/collection.rs`: a collection plugin that is switched on while a master its TES4 header names is off ("Master not switched on: MoreCraftableEquipment_USSEP.esp needs Unofficial Skyrim Special Edition Patch.esp") or loads below it ("Master loads after it"). Alias-aware, base game and Creation Club masters skipped, warn-only. Kill switch: rules `collection.masterCheck:false` (default on). No journal line of its own: the items land in the existing health row and its log line.

Checks: `cargo test -p launcher-core` 231 pass (1 new). Nothing in src-tauri touched. Not merged, released or game-verified. Effect on Timothy's PC UNKNOWN.

Plan (classification, USSEP and Creation Club handling, PC sequence): /mnt/project-files/aetherial-dawn/launcher/collection-rev7-plan.md. Spaced-USSEP/alias/MAST-rewrite and plugins.txt re-assert code belongs to the USSEP thread (PR 185 and follow-ups); this PR avoids aliases.rs, serverorder.rs and main.rs.
