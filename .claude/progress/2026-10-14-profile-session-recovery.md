# 2026-10-14 Preserve every unresolved mod-isolation session

Objective: ongoing Skyrim server GitHub bug audit. Baseline 122956e, isolated checkout A:/Aetherial Dawn/launcher-bug-audit, branch codex/server-bug-audit.

CONFIRMED by two new failing tests: (1) when an earlier session cannot restore a mod because a different file occupies its place, starting another session replaces the earlier journal; after the new session ends, pending() returns false and the old mod is never restored automatically. (2) rebuilding a corrupt journal from multiple folders keeps only the first folder with a conflict, losing tracking for the rest.

Fix: profile.rs reads the legacy single-session journal and a multiple-session envelope; writes retain the legacy form for one session. New set-aside operations retain earlier unresolved sessions, and both ordinary and rebuilt restores retain every remaining conflict. Existing replacement files are not overwritten. A repeated session folder is refused before any new file moves, preventing timestamp collisions from overwriting an earlier copy. Journal deletion errors now propagate instead of silently dropping them.

Validation on Windows/Node-independent Rust: first sandbox run failed because its default temp location denied renames; using checkout/.audit-tmp fixed the environment. Baseline plus new regressions: 9 pass, 2 fail at the predicted lost-tracking assertions. After implementation: profile suite passes. Final full `cargo test -p launcher-core --offline -- --test-threads=1`: 252 unit tests + 4 server-contract tests + 1 shipped-collection test pass, zero failures. git diff --check passes. Actual installed game/mod folders were not used.

No version bump, installer build, release, installation, game run or live mod-manager validation. Existing launchers that cannot parse the multiple-session envelope can use their default folder-rebuild path; this is unavailable if AD_ASIDE_REBUILD=0, so do not downgrade with pending multiple-session recovery and that override enabled. No file content migration is performed.

Wider audit remains IN_PROGRESS. Ingame PR #381 and Discord PR #95 now have green Linux CI (exact results recorded in their descriptions). Further follow-up: remaining direct JS short-write sites, profile path-validation review, launcher atomicfile durability and other source areas. Shared project records remain unavailable; this entry is the local handoff.

## Follow-up: validate recovery paths and preserve unlisted files

Three new regressions failed before this follow-up: a journal folder `../../victim` was accepted for cleanup, an extra list with `../outside.esp` moved an earlier valid file before reporting any problem, and restoring a session recursively deleted unlisted files in its folder. All reproduction targets were disposable fixtures inside the test temporary root.

profile.rs now validates the complete journal/request before restoration or new moves, rejects absolute/traversing/Windows-ambiguous/internal-metadata paths, and checks resolved existing ancestors for symlinks/junctions or escape from the selected game folder. Unsafe journals remain pending and return an error. Cleanup prunes empty directories only and does not follow directory links. Additional tests cover a later unsafe entry preventing earlier moves, path variants, and a Unix-only directory-link regression (requires Linux CI).

Final local Windows check: full core suite 257 unit + 4 contract + 1 collection tests pass (audit-core-paths.log), zero failures. The Unix symlink regression is not compiled on Windows. No actual game/mod folders used. Existing power-cut/fsync gaps remain a separate audit item; path checks do not claim protection against concurrent filesystem replacement by another process.
