# Plan: uninstall checks in a throwaway Windows (D18)

D18: uninstalling the launcher reverts nothing it changed. `src-tauri/windows/hooks.nsh`
has no `NSIS_HOOK_PREUNINSTALL`. This plan tests that **only in a disposable
Windows** (Windows Sandbox or a fresh VM). It never runs on Timothy's real PC,
real Skyrim, real Steam library or real Vortex profile.

## 1. What the launcher leaves on a PC (from the source)

| # | What | Where it comes from | Revert on uninstall? (proposal) |
|---|---|---|---|
| U1 | `appmanifest_489830.acf` set to `AutoUpdateBehavior 1` and made **read-only** | `core/src/version.rs` `hold_updates` | **Yes:** clear read-only. Timothy decides whether auto-update is put back too, since Steam would then update Skyrim past the server build. |
| U2 | Skyrim files changed to the server build (version fix) | `core/src/community.rs`, `patcher.rs` | **No.** The game stays on the server build. Steam's Verify brings back the newest. Tell the player this. |
| U3 | `<game>\.aetherial-dawn\` (version marker, `mods` records, `downgrade` work area, files set aside) | `version.rs` `MARKER`, `modlist.rs` `MODS_DIR`, `community.rs` `WORK_DIR`, `strays.rs` | **Ask the player:** put set-aside files back (they were the player's own mods), then remove the folder. |
| U4 | Mods the launcher put in `Data` (feed, SKSE, Crash Logger, Souls RE) | `modlist.rs`, `requirements.rs` | **Timothy decides:** leave them (safe) or remove the launcher-recorded ones only. Never touch files Vortex owns. |
| U5 | `plugins.txt` / `loadorder.txt` changed to the server's order, with `plugins.txt.aetherial-dawn-backup` | `loadorder.rs` `switch_off`, `serverorder.rs`, `aliases.rs` | **Yes, if a backup exists:** put the player's own list back (PR #8 keeps the first backup). |
| U6 | `Skyrim.ccc` renamed while playing | `serverorder::restore_ccc`, `main.rs` `CccGuard` | **Yes:** put it back if it is still renamed. |
| U7 | `%LOCALAPPDATA%\gg.aetherialdawn.launcher\` (settings, logs, cache, encrypted sign-in) | Tauri app data | **Yes:** remove it (sign-out). The installer's own "delete app data" box may already do this; to be checked. |

**Default taken: "Settings only"** (2026-09-30, while Timothy is away; he can
still change it): U1 clears read-only **and** puts Steam's auto-update back;
U2 no; U3 set-aside files go back, then the folder goes; U4 **leave** the
launcher's mods in Data; U5, U6, U7 yes.

## 2. Disposable setup
1. Use Windows Sandbox (Windows 10/11 Pro: Start, "Turn Windows features on or
   off", tick **Windows Sandbox**) or a fresh VM. Everything is thrown away on close.
2. Inside it, run `docs/qa/sandbox-fixture.ps1` (it refuses outside Windows
   Sandbox unless `-Disposable` is given, and refuses an existing folder). It
   builds a **fake Steam library** that has:
   `steamapps\appmanifest_489830.acf` (read-only, `AutoUpdateBehavior 1`),
   `common\Skyrim Special Edition\SkyrimSE.exe` (a dummy file),
   `.aetherial-dawn\` with a marker, a mod record and one set-aside file in
   `disabled\`, a
   `Data\` with two dummy plugins, and
   `%LOCALAPPDATA%\Skyrim Special Edition\plugins.txt` plus its
   `.aetherial-dawn-backup`, and a renamed `Skyrim.ccc` in the game folder. No real game files or
   keys are used.
3. Install the launcher build from the PR's CI artifact. Never use the public
   release channel, and never push to `main`. At first start choose
   `C:\ADTest\SteamLibrary\steamapps\common\Skyrim Special Edition` so its
   settings name the fake game, then close it without signing in.
4. Take snapshot A: every path, size, SHA-256 and read-only flag under the fake
   library, `%LOCALAPPDATA%\Skyrim Special Edition` and the launcher's app data
   with `docs/qa/snapshot.ps1 -Paths "C:\ADTest;$env:LOCALAPPDATA\Skyrim Special Edition;$env:LOCALAPPDATA\gg.aetherialdawn.launcher" -Out A.txt`
   (read-only: path, size, SHA-256, read-only flag).
5. Uninstall from **Settings > Apps**. Take snapshot B the same way (`-Out B.txt`).
6. Compare with `Compare-Object (Get-Content A.txt) (Get-Content B.txt)` and
   check each U-row against section 1: pass or fail.

## 3. Pass
- Today (live 0.1.87 and 0.1.98): expected **all fail** except U7 if the
  installer's box removes app data. That fail list is the D18 evidence.
- After an uninstall-cleanup fix: each row Timothy marks "Yes" is reverted.
  Rows marked "No" are unchanged, and nothing outside section 1 changed.

## 4. Not in this plan
- No code change yet. The fix (a `--uninstall-cleanup` mode run from
  `NSIS_HOOK_PREUNINSTALL`) follows the "Settings only" default above.
- The Vortex route (Packages A and C) is not touched here.
