# Aetherial Dawn launcher

This is the Windows launcher for the Aetherial Dawn Skyrim multiplayer (SkyMP) server. It does five things:
1. Finds Skyrim Special Edition and checks for SKSE.
2. Downloads the SkyMP client files that changed on the server.
3. Writes the SkyMP client settings.
4. Starts the game through SKSE.
5. Keeps itself up to date.

It's built with [Tauri 2](https://tauri.app). The UI is plain HTML, CSS and JS in `ui/`, and the Rust side is in `src-tauri/`. The logic that doesn't depend on Windows is in `core/` and is unit tested.

## Layout

| Path | What |
|---|---|
| `core/src/manifest.rs` | Server file list format and path safety checks |
| `core/src/sync.rs` | Hash comparison with a cache, verified downloads and removals |
| `core/src/settings.rs` | Writes `Data/Platform/Plugins/skymp5-client-settings.txt` |
| `core/src/game.rs` | Steam library detection, SKSE check, launching `skse64_loader.exe` |
| `src-tauri/src/main.rs` | Commands the UI calls, and saved settings |
| `ui/` | Launcher screens. Fonts are bundled so the launcher works offline |

## What the server publishes

The server serves static files under one base URL, for example `https://vps-d38c928e.vps.ovh.us/launcher`. The full format is in `skymp-setup/launcher/launcher-spec.md` in the project files.

- `client/manifest.json` and `client/files/<sha256>`. Build these with `make-manifest.py` from the SkyMP client build folder. Both this and `mods.json` must have a `.sig` next to them (`sign-feed`, [docs/signing-feeds.md](docs/signing-feeds.md)); launchers that have seen a signature refuse the file without one.
- `app/latest.json` and the installer (optional): a second source for launcher self-updates. The first is the latest GitHub release of this public repo.
- `status.json` (optional), which feeds the side panel: `{ "online": true, "players": 7, "maxPlayers": 100, "sinceReset": "1d", "news": [{ "date": "26 Sep 2026", "title": "…", "body": "…" }] }`
  Optional `launcher` block: `{ "hold": ["0.1.120"], "holdUntil": "2026-10-09T12:00:00Z", "minVersion": "0.1.115", "message": "…" }`. A hold pauses installing those launcher versions until `holdUntil` (ignored without it, so it can't last for ever); `minVersion` only shows `message` on Home, it never blocks Play.

## Building

Windows installers are built by `.github/workflows/build.yml`. Before a real release:

1. Set the repository variable `AD_BASE_URL` to the server's launcher URL.
2. Create an update signing key on your own PC by running `cargo tauri signer generate -w ~/.tauri/aetherial.key`. Put the private key and its password in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Never paste them into chat.
3. Put the public key in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`. `endpoints` lists this repo's latest GitHub release first and `<AD_BASE_URL>/app/latest.json` second. CI attaches a signed `latest.json` to every release when the signing secrets are set.
4. Replace the placeholder icons in `src-tauri/icons/`, then run `cargo tauri icon your-logo.png`.

To build locally on Windows, run `cargo install tauri-cli --version "^2"`, then `cargo tauri build`.

To cross-build the Windows installer from Linux, as used for the first test build, install `gcc-mingw-w64-x86-64`, `nsis` and `cargo install tauri-cli`. Then run `rustup target add x86_64-pc-windows-gnu` and `AD_BASE_URL=<url> cargo tauri build --target x86_64-pc-windows-gnu --bundles nsis --config '{"bundle":{"createUpdaterArtifacts":false}}'`. Tauri calls this cross-build experimental, and the installer isn't code-signed.

To test the file sync against any server, run `cargo run -p launcher-core --example sync -- <base-url> <skyrim-folder>`.

## Game version check

The server's `manifest.json` names the Skyrim build it needs (`game.version`, now 1.6.1170.0 with SKSE 2.2.6) and the Steam depot manifests for it. `core/src/version.rs` compares the player's SkyrimSE.exe, Steam's `appmanifest_489830.acf` and the launcher's own record. A wrong build is fixed by patching the player's own files ("Patching the game instead of downloading it" below), after a Steam "verify files" repair when the game is Steam's newest build. The launcher has no Steam downloader: DepotDownloader, Steam sign-in inside the launcher and the Steam console download were removed because nothing used them. `cargo run -p launcher-core --example gamever -- SkyrimSE.exe` prints an exe's version.

## Discord sign-in

The launcher signs players in through the login service at `AD_AUTH_URL` (default `https://vps-d38c928e.vps.ovh.us/ad`), following `aetherial-dawn-discord/CONTRACT.md` in the project files. The token is saved encrypted for the Windows user (DPAPI) and checked at start and every 10 minutes. A ban, or leaving the Discord, signs the player out. Each Play asks for a fresh game session and writes it into the SkyMP client settings, along with the client's remembered login in `Data/Platform/PluginsNoLoad`.

## Patching the game instead of downloading it

When the game is Steam's current build (SkyrimSE.exe 1.7.104, checked by SHA-1), Fix version uses the free community patches from MulderLoad's Skyrim SE Steam Downgrader (github.com/Mulderland/MulderLoad), approved by Timothy on 2026-09-26 (`core/src/community.rs`). It downloads MulderLoad's 1.7.104-to-1.6.1170 archives (plus the language pack when the game isn't English) from cdn.mulderload.eu and checks them against the SHA-1s MulderLoad pins. It unpacks them into `.aetherial-dawn/downgrade/` on the game's drive, and applies each `.xdelta` with the official xdelta3 3.0.11 (jmacd/xdelta-gpl, SHA-1 checked, run with no window). Every new file is made before any is swapped in. Afterwards it clears `Data/ShaderCache` and renames a new-format ContentCatalog.txt, as MulderLoad does. On 2026-09-26 the server confirmed these patches turn the 1.7.104 masters into exactly the server's 1.6.1170 masters. Other builds use the server's own patches, described below.

Fix version patches the player's own Skyrim files into the server's build, with no Steam download or sign-in (`core/src/patcher.rs`, 0.1.26). The server publishes `<base>/patches/index.json`. For each Steam game file (exe, base masters, Bethesda archives, the Creation Club files that come with the game, `Skyrim.ccc`) it lists the target size and SHA-256, and patches keyed by the SHA-256 of the copy they start from. Patches are zstd "patch-from" deltas, useless without the player's own copy of the file. The launcher hashes the player's files, downloads only the patches it needs, and writes each result next to the original. It checks the SHA-256 before swapping the file in, then records the build, holds Steam updates and keeps its copy, as after any downgrade. When the player's copy is a build with no patch, it says so and offers the Steam options under **Other ways to download**.

To make patches, run the launcher as `AetherialDawn.exe --make-patches <newer game folder> <server-build game folder> <out folder> [1.6.1170.0]`. This writes the `.zst` files and `index.json` to the out folder. It proves each patch round-trips, logs to `make-patches.log`, and merges with an existing index, so one folder can serve several Steam builds. Upload the folder to the server's `launcher/patches/`.

Patches made on a staff PC can be put in `<game>/.aetherial-dawn/patch-build/out`; Fix version on that PC uses them directly when their target matches the server (there is no in-launcher builder any more, so make them with `--make-patches`).

## Putting the game back by itself

After a downgrade the launcher keeps hard links to Steam's own files (the exe, `steam_api64.dll`, `bink2w64.dll`, the base masters, Bethesda's and Creation Club archives, and `Skyrim.ccc`) in `.aetherial-dawn/pristine/` inside the game folder (`core/src/pristine.rs`). This takes no extra space. When Steam updates or repairs Skyrim, it writes new files and leaves the links on the old build. Every version check (at start, on Check, and before Play) then puts the kept files back, marks the build, and holds Steam updates again, with nothing for the player to do. Players who downgraded before 0.1.21 get the copy on their next check. If the drive can't hold hard links (FAT32 or exFAT), or a file was changed in place, there is no usable copy and the launcher asks for a download as before.

## Crash reports

After Play the launcher hides instead of closing and watches `SkyrimSE.exe`. When the game ends it saves `game-<time>.txt` next to the launcher log: how long Skyrim ran, its exit code in plain words, every log written that session under `Documents/My Games/Skyrim Special Edition/SKSE` (including crash logger output) and SkyrimPlatform's temp folder, and the last 40 launcher log lines. A crash (a Windows exception code, or the game closing within 90 seconds) brings the launcher back with the report and a Copy report button; a normal quit closes the launcher. Copy diagnostics includes the latest report.

## Plugins from other mods

After each check, the launcher lists every `.dll` in `Data/SKSE/Plugins` and every file in `Data/Platform/Plugins` that isn't in the server's file list (the SkyMP settings file aside). Leftover plugins like these crashed the game before the main menu in the first live test. Play turns into **Check mods**, which shows the list and moves the files to `.aetherial-dawn/disabled/<time>/` inside the game folder, keeping their paths. Nothing is deleted. Players can also choose to play with them anyway, which is logged.

Check mods also reads the load order (`%LOCALAPPDATA%\Skyrim Special Edition\plugins.txt`). Any plugin switched on there that isn't the base game, `_ResourcePack.esl`, listed in `Skyrim.ccc` (Creation Club) or shipped by the server is listed, and so is a broken plugin file (bad header, version 0, no records). **Fix it** switches them off in plugins.txt. The first time the launcher changes plugins.txt or loadorder.txt it keeps the old file as `plugins.txt.aetherial-dawn-backup` (or `loadorder.txt.aetherial-dawn-backup`); later changes never replace that copy, so it stays the player's own list. The plugin files stay in Data. Vortex rewrites plugins.txt when it deploys, so Vortex users also switch the plugin off in Vortex's Plugins tab. In the first live test, a 59-byte `SkyUI_SE.esp` stub from an old setup was the only active plugin.

## Tidying before every Play

Players don't click anything for this. Before each launch, the launcher:
- moves SKSE/Platform plugins and loose `Data/Interface` files the server didn't ship (old RaceMenu, map and HUD menus) to `.aetherial-dawn/disabled/<time>/`;
- switches extra or broken plugins off in plugins.txt;
- drops archives from `sResourceArchiveList`/`sResourceArchiveList2` in Skyrim.ini, SkyrimPrefs.ini and SkyrimCustom.ini that no longer exist in Data, plus repeated ones. The original ini is kept once as `<name>.aetherial-dawn-backup`.

Everything it changes is written to the log. In the first live test, Skyrim.ini still named 43 BSAs from uninstalled mods.

## Plugins for a newer Skyrim

Steam updates Creation Club downloads separately from the game, so after a downgrade they can stay on the newer build. Before every Play, `loadorder::too_new` compares each plugin's header version and form version with the five base masters. Any plugin that is newer, and any broken stub plugin, is moved with its archives to `.aetherial-dawn/disabled/<time>-plugins/`. The game skips Creation Club files that aren't in Data. The version record and the kept copy are then refreshed. Skyrim Platform's browser cache (`%TEMP%\\Skyrim Platform`) is cleared, because a stale profile there crashed libcef.dll 5 seconds in. A crash logger that launchers before 0.1.20 moved aside is put back, so the next crash names the failing module. Crash reports to staff go out in the background and are retried once when the staff service asks for a short wait.

## Plugin names the game client can't load

SkyMP's client checks the load order by calling Skyrim Platform's `getFileInfo` for every plugin, and that rejects names with spaces: on 2026-09-26 Timothy's `skyrim-platform.log` ended with "'unofficial skyrim special edition patch.esp' is not a valid argument for 'filename'", and the game sat on a black screen after loading in.

Since 0.1.49 (Timothy: correct it, don't switch it off), before every Play `aliases::ensure` gives each plugin whose name has anything but letters, digits, `_`, `-` and `.` a hard link (a copy if linking fails) under a dash-joined name, `Unofficial-Skyrim-Special-Edition-Patch.esp`, together with the files keyed to its name: `<name>.bsa`, `<name> - Textures.bsa`, `<name>.ini`, `Strings/<name>_*` and `Interface/Translations/<name>_*`. plugins.txt and loadorder.txt name the copy where the original was, switched on or off as the original was. The original is never renamed or moved (Vortex owns it); a copy whose original is gone is removed, and a copy is refreshed when its original changes. The links are recorded in `.aetherial-dawn/mods/aliases.json` and count as the original mod's files for tidying. The game check lists them under Plugin names as information. A plugin that names such a plugin as a master would still look for the original name; none of the server's mods do.

## Game health checks

The checks Claude ran by hand on the first tester's PC run for every player: before each Play, after a crash, and from **Settings, Check my game**. They cover:
- the SkyrimSE.exe version
- the five masters against `<base>/masters.json` from the server (size and SHA-256, cached by size and date)
- plugins switched on in plugins.txt, and loadorder.txt sanity
- stub or broken plugin files
- Skyrim.ini archive lines that point at missing BSAs
- stray SKSE plugins, Platform scripts and loose menus
- injector DLLs next to the exe
- known overlays that are running
- Steam's auto-update setting and hold
- whether a crash logger is installed

Each check shows OK, INFO, WARN or FAIL. The crash report includes the results. Reports go to staff before Play when something is at WARN or worse, and after every crash. The player can turn this off in Settings, and the Check my game screen shows the exact JSON that is sent. The report has the Discord id and name, the launcher and server build, and the check results. It never includes the settings file, tokens or sessions, and the home folder is written as %USERPROFILE%. Reports go to `<AD_AUTH_URL>/api/crash-reports` with the launcher token and `consent: true` (`HEALTH_REPORTS_ON` in main.rs, on since 0.1.22). A 404 is only logged. The body is kept under 60 KB and never names the client settings file or PluginsNoLoad. After a crash, the crash window shows the staff report number and the likely cause the server sends back, and the report includes the crash logger's exception and call stack when one is installed. Crash loggers (CrashLogger.dll, TrainwreckSKSE.dll, NetScriptFramework) are never moved aside.

## Launcher updates

The launcher installs new releases by itself, without asking. It checks at startup and every minute (and from the Check for updates button in Settings), but never while Skyrim is running or a download is in progress.

## Keeping the game on the right build

After a downgrade (either way), the launcher sets `"AutoUpdateBehavior" "1"` (only update when launched) in `steamapps/appmanifest_489830.acf` and makes that file read-only, so Steam doesn't swap the game data back to the newer build. The game is always started through SKSE, never through Steam. Server Info always offers **Fix version** / **Re-download** when the server lists depots. **My game is already on this version** needs a second click, and the mark it leaves is removed automatically if Skyrim then crashes, so the version check runs again. In the first live test, a player used it over updated game data and Skyrim crashed with an access violation after 4 seconds.

## Reporting problems

The launcher keeps a log at `%LOCALAPPDATA%\gg.aetherialdawn.launcher\logs\launcher.log` (the previous one is kept as `launcher.old.log` once it passes 2 MB). It records startup, every command and how it ended, sign-in results, the downgrade steps and DepotDownloader arguments, and script errors. It never records the Discord token, passwords or game sessions. **Settings, Copy diagnostics** puts a report on the clipboard: launcher version, Skyrim exe version, SKSE files, Steam depots, the version check, server build, sign-in state and the last 80 log lines. **Open log folder** opens the log folder.

## Known gaps

- **Discord sign-in:** built to aetherial-dawn-discord/CONTRACT.md and tested against a stand-in service, not yet the live one. The file name of the game's remembered login (`auth-data-no-load.js`) is inferred from the SkyMP client source and needs checking on the first real test.
- **Game detection:** only Steam installs are found automatically. GOG and other installs use the folder picker.

Fonts are Cinzel, Hanken Grotesk and JetBrains Mono, all under the SIL Open Font License.

## Mods

The launcher no longer lists, downloads, installs, sets aside or checks third-party mods, and there is no Mods page, Vortex or Nexus step. It keeps only what SkyMP needs: the game version check and fix, SKSE, the SkyMP client files from `client/manifest.json`, the server's plugin order from `masters.json`, Discord sign-in and the game health checks. Old launchers still read `mods.json`; the server keeps an empty one for them.

## Menu music

Timothy (2026-09-26): quiet Skyrim music in the launcher. Bethesda's music can't ship with the launcher, so `core/src/bsa.rs` reads the main title theme (or an explore track) straight from the player's own `Data/Skyrim - *.bsa` (version 105, LZ4 when compressed) into memory, and `src-tauri/src/music.rs` plays the xWMA with Windows' XAudio2 at 12% volume with a 3-second fade in, looping. Nothing is copied out of the game folder. It stops when Play starts the game. The first time, a small card asks Keep music / Mute; the answer is saved (`music` in the launcher config) and Settings has a Menu music switch. Without a Skyrim folder there's no music.

## Discord patch notes
When a `vX.Y.Z` release is published, the release job posts player patch notes to Discord (the PRs merged since the previous release, grouped, in plain words) and a line to the staff channel; a failed release also posts to the staff channel. Add two repository secrets, each a Discord channel webhook URL: `AD_PATCHNOTES_WEBHOOK` (player channel) and `AD_STAFF_WEBHOOK` (private staff channel). With none set the steps print "not posted" and the release is unaffected.
Wording: add `Patch note: Play no longer stalls on the status line.` to the PR description (several lines make several bullets; `Patch note: none` hides the PR). Without one the PR title is cleaned up and used. Preview: `node .github/scripts/patch-notes.js notes --from v0.1.103 --to HEAD`. Tests: `node --test .github/scripts/patch-notes.test.js`.
