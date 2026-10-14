# 2026-10-14 Unreadable hash cache (files.json) no longer forgets which files the launcher put there

Bug hunt, backlog item "launcher core sync.rs reads an unreadable hash cache as empty".

CONFIRMED by failing tests (run with AD_SYNC_CACHE_RECOVER=0, the old behaviour): `.aetherial-dawn/files.json` (hash cache and the record of launcher-placed files, which `removable` needs before the server's remove list may delete anything) that was cut short or zeroed (atomicfile::write has no fsync) was read as empty and overwritten by the next save, so stale mods named on the remove list stayed for good.

Change (core/src/sync.rs only; src-tauri and deploy untouched, no Windows build needed beyond the normal release): save keeps the last three copies (`safe_write`); an unreadable cache is kept as `files.json.corrupt-<ms>`, the newest readable `.bakN` is used, and a stderr line says which. With no readable copy nothing is removed (unchanged safe default) and the bad file is kept. Kill switch: env AD_SYNC_CACHE_RECOVER=0.

Offline-tested: 2 new sync tests, core suite 251 pass. NOT done: download size cap (hardening only, INFERRED, no failing test; hash check already rejects wrong bodies). Not game-verified; no restart needed on the server; reaches players with the next launcher release.
