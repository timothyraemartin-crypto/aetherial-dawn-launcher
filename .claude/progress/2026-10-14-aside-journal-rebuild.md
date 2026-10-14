# 2026-10-14 Set-aside journal: an unreadable journal no longer strands the player's mods

Bug hunt, launcher core/src/profile.rs. CONFIRMED by failing tests: if `.aetherial-dawn/aside/journal.json` exists but does not parse (zeroed or cut short by a power cut, since atomicfile::write does not fsync; or edited), `read_journal` returned None, so `pending()` said nothing was waiting, `restore()` did nothing, and the next `set_aside` wrote a new journal over it. The player's mods stayed in `aside/<stamp>/` with no record and were never put back.

Fix: an unreadable journal makes `pending()` true when session folders hold files, and `restore()` rebuilds the list from those folders (same never-overwrite rule, identical copies dropped, differing ones stay listed). Switch: env AD_ASIDE_REBUILD=0 restores the old behaviour. Log: stderr line only (core has no logger); src-tauri not changed.

Tests: 3 new in profile.rs; `cargo test -p launcher-core` 239 pass. Needs a launcher release. Not game-verified. INFERRED (not fixed): atomicfile::write has no fsync; kept files from an earlier session drop out of the journal when the next set_aside writes a new one.
