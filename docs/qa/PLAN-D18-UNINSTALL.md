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
| U8 | The game's remembered login and session: `Data\Platform\PluginsNoLoad\auth-data-no-load.js`, and the `session` in `Data\Platform\Plugins\skymp5-client-settings.txt` (the server address, keys and the rest of that file stay) | `main.rs` `play` (`settings::write`, `settings::write_auth_data`) | **Yes:** forget them, as signing out does (`settings::clear_login`). |
| U9 | `Skyrim.ini`, `SkyrimPrefs.ini`, `SkyrimCustom.ini` in `Documents\My Games\Skyrim Special Edition` cleaned of missing or repeated archive lines, with `<name>.aetherial-dawn-backup` kept on the first change | `gameini.rs` `repair`, called from `main.rs` before Play | **No:** the repaired files only lost lines naming archives that aren't there; the backups stay so the player can put the old ones back. |
| U10 | Windows' graphics-card preference for `SkyrimSE.exe` (`HKCU\Software\Microsoft\DirectX\UserGpuPreferences`, set only when there was none) | `game.rs` `prefer_fast_gpu` | **No:** harmless, and the player may rely on it. |
| U11 | Skyrim Platform's empty plugin folders | `game.rs` `ensure_platform_folders` | **No.** |

**Default taken: "Settings only"** (2026-09-30, while Timothy is away; he can
still change it): U1 clears read-only **and** puts Steam's auto-update back;
U2 no; U3 set-aside files go back, then the folder goes; U4 **leave** the
launcher's mods in Data; U5, U6, U7, U8 yes; U9, U10, U11 no. PR #39 does
exactly this: it changes U1, U3, U5, U6 and U8, U7 is the installer's box, and
it touches nothing else. U1 changes only a manifest the launcher itself held
(`version::made_by_launcher`).

## 2. Disposable setup
1. Use Windows Sandbox (Windows 10/11 Pro: Start, "Turn Windows features on or
   off", tick **Windows Sandbox**) or a fresh VM. Everything is thrown away on close.
2. Inside it, build a **fake Steam library** with the fixture script from the
   follow-up tooling PR. It has:
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
   library, `%LOCALAPPDATA%\Skyrim Special Edition` and the launcher's app data,
   with the read-only snapshot script from the follow-up tooling PR.
5. Uninstall from **Settings > Apps**. Take snapshot B.
6. Compare A and B against section 1. Each U-row is pass or fail.

## 3. Pass
- Today (live 0.1.87 and 0.1.98): expected U1, U3, U5, U6 and U8 **fail**;
  U2, U4 and U9-U11 are unchanged; U7 depends on the installer's box. That
  result is the D18 evidence.
- A launcher update and a one-click reinstall over the same version must
  change none of U1-U11 (PR #39 skips the cleanup for `/UPDATE` and `/P`).
- After an uninstall-cleanup fix: each row Timothy marks "Yes" is reverted.
  Rows marked "No" are unchanged, and nothing outside section 1 changed.

## 4. Not in this plan
- No code change yet. The fix (a `--uninstall-cleanup` mode run from
  `NSIS_HOOK_PREUNINSTALL`) is proposed only after a disposable Windows run,
  and follows whatever Timothy picks for U1, U2 and U4.
- The fixture, snapshot and compare scripts live in a separate tooling PR, so
  this plan stays documentation only.
- The Vortex route (Packages A and C) is not touched here.
