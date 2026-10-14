# 2026-10-14 Isolate and sync atomic file writes

CONFIRMED: the fixed `<path>.tmp` filename allowed concurrent saves to steal one another's temporary file and allowed a pre-existing hard link to overwrite an unrelated file. Two regression tests failed against the original writer (audit-atomic-red-outside.log); both pass with exclusive, randomly named sibling temporary files. Only the writer's own temporary file is removed on failure. File contents are written completely and synced before replacement.

Validation: `cargo test -p launcher-core --offline -- --test-threads=1`, with workspace TEMP/TMP and filesystem/network permissions for fixtures, passed 259 unit tests, 4 contract tests, and 1 collection test on Windows (audit-core-atomic.log). The focused seven-test suite also passes. Sync failure injection verifies the old destination survives and the owned temporary file is cleaned. No new dependency was added.

NOT verified: real power-cut durability, installed launcher/game behavior, or Linux CI for this follow-up. Parent-directory metadata is not synced, and concurrent backup-history rotation is not serialized. No merge, release, install, or live game-file operation occurred. Next: obtain CI for the pushed revision and continue the wider GitHub audit.
