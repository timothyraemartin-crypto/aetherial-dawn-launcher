# 2026-10-11 Play now checks for a newer launcher release first

Play now checks for a newer launcher release first (ui/app.js updateBeforePlay): installs and relaunches before the game starts, 10 s timeout, offline/failed check logs and starts with the current version, staff holds still apply. New text: "Updating the launcher to X before the game starts…" (needs Timothy's OK before merge). Checked: ui-fast-play-test (3 new scenarios), ui-contract-test. Not built as Windows app; real Play-time update NEEDS_GAME_CHECK. Not released.
