# 2026-10-14 Mod pack pre-flight (draft PR claude/modpack-preflight-dsbjhi)
Timothy 00:55Z: wants to play-test mod packs easily and fast. Added `pack-preflight` (scan / triage / addrlib), `core/src/preflight.rs`, `core/src/addrlib.rs`, `core/data/dll-ledger.json`, `scripts/pack-fetch.sh`, workflow `Pack pre-flight`, `docs/pack-playtest.md`, decision D-2026-10-14-1. Also `skse::uses_address_library`.

Checks passed (offline): `cargo test -p launcher-core` 244 pass incl. 8 new; clippy clean for the new files; smoke run of the binary on an empty folder. FINDING (CONFIRMED by grep): `skse::wrong_builds/build_of` had no caller in core or src-tauri, so the old-build DLL check never ran at Play.

NOT verified: the workflow and `pack-fetch.sh` end to end (no Nexus key in the cloud box; first run is the check); the Address Library reader against a real versionlib file (format INFERRED from CommonLibSSE); nothing game-verified; launcher unchanged, no release, no deploy.
Next: run the workflow on rev 7 and read the report; seed the ledger from the first trial launch's skse64.log; then the trial-launch button and Play-time row (docs/pack-playtest.md).
NEEDS_GAME_CHECK: one trial launch of a pack, then `pack-preflight triage` on its skse64.log, to confirm the ledger lines match what the game did.
