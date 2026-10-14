# 2026-10-14 aliases.json not saved when ensure_full stops part-way

CONFIRMED by failing test (core/src/aliases.rs `a_failed_later_plugin_does_not_make_the_earlier_copy_look_foreign`): `ensure_full` saved its record of launcher-made links and rewritten copies only after a clean run. When a later plugin failed (a player's own dash-named file in the way), the copy made for an earlier plugin (e.g. `_ResourcePack.esm`) stayed on disk with no record; the next run called it "a different _ResourcePack.esm" and refused every Play until the file was removed by hand.

Change: the record is saved when the run stops with an error too (kill switch env AD_ALIASES_SAVE_ON_ERROR=0 = old behaviour; warn line on stderr "aliases: stopped part-way"), and aliases.json is written tmp+rename (AD_ALIASES_ATOMIC_SAVE=0 = old plain write). INFERRED, not fixed: an unreadable aliases.json still reads as empty (same class as sync.rs files.json, #196).

Offline-tested: core 252 pass. Not game-verified; restart of nothing needed, ships with next launcher build.
