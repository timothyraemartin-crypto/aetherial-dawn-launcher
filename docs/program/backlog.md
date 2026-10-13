# Prioritized backlog: launcher

Baseline 2026-10-13. Tiers follow Standing Goal section 3. Items are findings from the baseline inventory (labels per item) plus open PRs. Preserve every item; close one only with evidence, and note the closing PR here. Program-level items (Server Quality Plan, memory asks, fix list) are in `/mnt/project-files/aetherial-dawn/program/backlog.md`.
Item IDs: LA-F# = launcher finding F-#.

## Findings by tier

### Tier 1: item/currency duplication or loss, data corruption, unauthorized access, crashes

- **LA-F1** Tier 1. masters.json (main.rs:1971 plain GET) and patches/index.json (core/src/patcher.rs, no feedsig call) are unsigned, yet masters.json drives file downloads into Data (main.rs:1796-1818) and the patch index sets target SHA-256 of game files: a server/TLS compromise can push arbitrary game files to every player. CONFIRMED code; signed-feeds memory already lists "Not covered: masters.json, patch files" (REPORTED).
- **LA-F2** Tier 1 (resolved in code). plugins.txt wipe with no backup (quality-reviews Plugin order #1 / PR #54): read_text + atomic_write + .aetherial-dawn-backup/-previous now in core/src/loadorder.rs:235,283-289 (CONFIRMED). Close the memory item after a real-game check.

### Tier 2: broken core gameplay and blocked integrations

- **LA-F4** Tier 2. Play sequence untested and uncompiled off Windows: main.rs play_run has no unit test; PROGRESS repeatedly "src-tauri NOT compiled here". 0.1.143 shipped a blocking gate that stopped Play (CONFIRMED PROGRESS 2026-10-13). Rule to keep: new checks warn-only; add Play-sequence integration test (quality-reviews-4 open item "Play sequence has no unit tests").
- **LA-F5** Tier 2. Every Play change since 0.1.134 is "not game-verified" (PROGRESS). 0.1.146/0.1.147 need PC check: join, one mod missing, plugin off, game-temp path, no "another server's script" line.
- **LA-F6** Tier 2. Unknown cause of Lambcut-class "never connects" (in-game browser not starting): INFERRED file/AV problem; only preventive checks exist (browsercheck.rs, ladder.rs). Blocked pending data.
- **LA-F18** Tier 2. Installed-version drift is invisible to staff: the bot counts launcher versions from x-launcher-version (discord health/launcher.js) but old launchers (<= 0.1.143) still run the blocking gate; no server-side minimum version enforcement (decisions.md D-2026-10-11-1 limit, CONFIRMED).

### Tier 3: reliability, recovery and performance

- **LA-F3** Tier 3. feedsig.rs:35 `REQUIRE_SIGNED = false`: fresh installs accept unsigned mods/manifest/rules (CONFIRMED). Rollout flip is a later release (REPORTED signed-feeds.md).
- **LA-F7** Tier 3. fetch_server_plugins (main.rs:1796-1822) no retry, 60 s timeout, whole file buffered; one 404 or hiccup stops Play with a message.
- **LA-F8** Tier 3. end_leftover_skyrim (main.rs:784) terminates any windowless SkyrimSE.exe older than 30 s when the launcher has no session; may end a player's Skyrim started elsewhere (INFERRED). Consider requiring that the process was started by this launcher.
- **LA-F9** Tier 3. launcher-rules publish mode left file 0600 -> 403 for everyone (root-causes 2026-10-13). Script fixed (#158) but "not yet run on the server" (REPORTED). Verify HTTP 200 after next rules publish.
- **LA-F10** Tier 3. Staff upload is a detached task without outbox (detach.rs): reports lost if the launcher exits before the upload completes (INFERRED).
- **LA-F11** Tier 3. quality-reviews-2 SkyMP file sync still open in code: no retry on failed download, no size cap, first failure aborts (sync.rs:220-262, CONFIRMED); `apply`/`download_verified`/`replace` lack direct tests (sync.rs tests are plan/remove oriented).
- **LA-F12** Tier 3. manifest::safe_relative (manifest.rs:111) does not reject Windows reserved device names, trailing dots/spaces or case collisions (quality-reviews-2 Medium, CONFIRMED still open).
- **LA-F13** Tier 3. quality-reviews Plugin order #2 (health.rs reports OK on an unusable masters list): Play now stops via valid_base (serverorder.rs:137, main.rs:410) but health.rs behaviour itself not re-verified (UNKNOWN). #3 (list_hash pin test) and Vortex bridge/extension findings (quality-reviews-3, vortex-extension-review #1-#5) refer to deleted code: close as obsolete (INFERRED). #4 (MAX_FULL 254 not enforced) not found in serverorder.rs by grep (still open, INFERRED).
- **LA-F20** Tier 3. Installed-state unknowns to resolve before trusting this register: Timothy's PC runs 0.1.146 (REPORTED), other players UNKNOWN; VPS rules file revision served UNKNOWN; whether native QuietPickups DLL loads in game UNKNOWN.

### Tier 4: usability and presentation

- **LA-F14** Tier 4. Docs drift: README "Known gaps" (sign-in tested only against stand-in), "Game detection", launcher-parts.md (vortex-extension, Steam downloader), live-mod-list.md and cross-repo/mod-list.md (mods.json, modlist.rs), repo-links.md (contract PRs open) are stale (CONFIRMED against tree).
- **LA-F15** Tier 4. collection-refresh.yml hard-codes revision '6' as default (lines 25, 34); must be bumped by hand.
- **LA-F16** Tier 4. UI items from quality-reviews (progress listener outside try so `busy` can stick; settings toggles not rolled back when set_prefs fails; unreachable "launcher is updating" message; news cannot be cleared; stale player counts after failed fetch) not re-verified (UNKNOWN).
- **LA-F17** Tier 4. quality-reviews-4: rustfmt report-only; src-tauri tests in CI status UNKNOWN (#59 not confirmed merged by me; ui-contract-test.js exists CONFIRMED).

### Tier 5: approved feature expansion

- **LA-F19** Tier 5. Approved expansion candidates in memory: launcher-improvements A sync staging, B health sheet fixes, C put my setup back (default ON, partly built as the profile), D crash advisor (REPORTED launcher-improvements.md); best-ideas.md has 7 more (ideas only).

## Open pull requests (as of 2026-10-13 15:15Z; state changes fast, re-check before acting)

| # | State | Title | Note |
|---|---|---|---|
| 162 | draft | Client parity: join fingerprint and "same game files as the server" row | Server Quality Plan thread 4; needs CI compile of main.rs edits, PC check and release phrase; receiver is discord 42 |
| 149 | draft | root-causes: archive list wrong for 1.6.1170 | docs only |
| 134 | draft | Fast Play UI test: wait for PLAY before pressing (fixes #129) | auto-fix PR |
| 112 | draft | Publish launcher feed: strip-mods modes | HOLD until a mod-free launcher is released |
| 111 | draft | Remove Vortex docs, extension CI steps and review notes (slice A) | possibly obsolete: launcher dropped the Vortex bridge (REPORTED by inventory); triage |
| 87 | draft | Sign release: start from empty dist/ on the persistent runner | auto-fix PR for #85 |
| 69 | draft | Portrait snapshot: groundwork and capture test | needs real-PC capture test |
| 51 | ready | Remove two unreferenced UI images | old (2026-10-07), verified PASS |
| 39 | draft | Uninstalling the launcher puts back what it changed | base branch cutover-098; needs Windows Sandbox receipt; Timothy decisions U1/U2/U4 |
| 35 | draft | QA D18 throwaway-Windows scripts | base qa-uninstall-plan |
| 33 | draft | QA read-only Windows evidence script (D13, D14) | base cutover-098 |
