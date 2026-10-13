# 2026-10-13 Client parity running readings

Client parity running readings (same PR #162): after the game starts the launcher reads plugins.txt again at 45 s and 180 s (`phase:"running"`, only while the game runs) so a rewrite after launch (seen ~12 s after start on Timothy's PC, REPORTED) shows as parity.mismatch/DRIFT on the server. core 224 tests pass, clippy clean; the main.rs change is uncompiled here and Windows CI jobs skip PRs. NOT merged or released; needs the PC check and his typed phrase.
