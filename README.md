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

- `client/manifest.json` and `client/files/<sha256>`. Build these with `make-manifest.py` from the SkyMP client build folder.
- `app/latest.json` and the installer (optional): a second source for launcher self-updates. The first is the latest GitHub release of this public repo.
- `status.json` (optional), which feeds the side panel: `{ "online": true, "players": 7, "maxPlayers": 100, "sinceReset": "1d", "news": [{ "date": "26 Sep 2026", "title": "…", "body": "…" }] }`

## Building

Windows installers are built by `.github/workflows/build.yml`. Before a real release:

1. Set the repository variable `AD_BASE_URL` to the server's launcher URL.
2. Create an update signing key on your own PC by running `cargo tauri signer generate -w ~/.tauri/aetherial.key`. Put the private key and its password in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Never paste them into chat.
3. Put the public key in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`. `endpoints` lists this repo's latest GitHub release first and `<AD_BASE_URL>/app/latest.json` second. CI attaches a signed `latest.json` to every release when the signing secrets are set.
4. Replace the placeholder icons in `src-tauri/icons/`, then run `cargo tauri icon your-logo.png`.

To build locally on Windows, run `cargo install tauri-cli --version "^2"`, then `cargo tauri build`.

To cross-build the Windows installer from Linux, as used for the first test build, install `gcc-mingw-w64-x86-64`, `nsis` and `cargo install tauri-cli`. Then run `rustup target add x86_64-pc-windows-gnu` and `AD_BASE_URL=<url> cargo tauri build --target x86_64-pc-windows-gnu --bundles nsis --config '{"bundle":{"createUpdaterArtifacts":false}}'`. Tauri calls this cross-build experimental, and the installer isn't code-signed.

To test the file sync against any server, run `cargo run -p launcher-core --example sync -- <base-url> <skyrim-folder>`.

## Game version check and downgrader

The server's `manifest.json` names the Skyrim build it needs (`game.version`, now 1.6.1170.0 with SKSE 2.2.6) and the Steam depot manifests for it. `core/src/version.rs` compares the player's SkyrimSE.exe, Steam's `appmanifest_489830.acf` and the launcher's own record. `core/src/downgrade.rs` fixes a mismatch by running DepotDownloader in its own window, where the player signs in with their own Steam account. The full format is in the spec.

To test without Steam, point `game.tool.url` at a zip holding a stand-in `DepotDownloader`. `cargo run -p launcher-core --example gamever -- SkyrimSE.exe` prints an exe's version.

## Discord sign-in

The launcher signs players in through the login service at `AD_AUTH_URL` (default `https://vps-d38c928e.vps.ovh.us/ad`), following `aetherial-dawn-discord/CONTRACT.md` in the project files. The token is saved encrypted for the Windows user (DPAPI) and checked at start and every 10 minutes. A ban, or leaving the Discord, signs the player out. Each Play asks for a fresh game session and writes it into the SkyMP client settings, along with the client's remembered login in `Data/Platform/PluginsNoLoad`.

## Signing in to Steam inside the launcher

The recommended downgrade option (0.1.21) runs DepotDownloader with no window of its own (`downgrade::spawn_piped`, `-remember-password`). `downgrade::Scanner` reads its output and turns the password prompt, Steam Guard code prompts (authenticator or email), the phone-approval notice and the download percentage into `steam-login` events. The Fix version window shows a password or code box when Steam asks, and `steam_login_answer` writes the answer to DepotDownloader's input. The answer is never logged or kept. Steam gives DepotDownloader a sign-in token, stored in the launcher's tools folder, so later downloads usually need no password. The older options (Steam console, QR code window, separate sign-in window) sit under **Other ways to download**.

## Putting the game back by itself

After a downgrade the launcher keeps hard links to Steam's own files (the exe, `steam_api64.dll`, `bink2w64.dll`, the base masters, Bethesda's and Creation Club archives, and `Skyrim.ccc`) in `.aetherial-dawn/pristine/` inside the game folder (`core/src/pristine.rs`). This takes no extra space. When Steam updates or repairs Skyrim, it writes new files and leaves the links on the old build. Every version check (at start, on Check, and before Play) then puts the kept files back, marks the build, and holds Steam updates again, with nothing for the player to do. Players who downgraded before 0.1.21 get the copy on their next check. If the drive can't hold hard links (FAT32 or exFAT), or a file was changed in place, there is no usable copy and the launcher asks for a download as before.

## Downgrading through the Steam app

The Steam console option uses the Steam app the player is already signed into. Steam doesn't let other programs start a depot download, so the launcher opens Steam's console (`steam://open/console`) and shows the three `download_depot 489830 <depot> <manifest>` lines with Copy buttons. Steam downloads each depot into `<Steam>/steamapps/content/app_489830/depot_<id>/`. The launcher watches those folders, and when all of them have been quiet for 10 seconds and the player clicks Install, it copies the files into the game folder, deletes the downloaded copy, and checks the version. The DepotDownloader options (Steam mobile app QR, or account name) stay as a fallback for when Steam isn't running.

## Crash reports

After Play the launcher hides instead of closing and watches `SkyrimSE.exe`. When the game ends it saves `game-<time>.txt` next to the launcher log: how long Skyrim ran, its exit code in plain words, every log written that session under `Documents/My Games/Skyrim Special Edition/SKSE` (including crash logger output) and SkyrimPlatform's temp folder, and the last 40 launcher log lines. A crash (a Windows exception code, or the game closing within 90 seconds) brings the launcher back with the report and a Copy report button; a normal quit closes the launcher. Copy diagnostics includes the latest report.

## Plugins from other mods

After each check, the launcher lists every `.dll` in `Data/SKSE/Plugins` and every file in `Data/Platform/Plugins` that isn't in the server's file list (the SkyMP settings file aside). Leftover plugins like these crashed the game before the main menu in the first live test. Play turns into **Check mods**, which shows the list and moves the files to `.aetherial-dawn/disabled/<time>/` inside the game folder, keeping their paths. Nothing is deleted. Players can also choose to play with them anyway, which is logged.

Check mods also reads the load order (`%LOCALAPPDATA%\Skyrim Special Edition\plugins.txt`). Any plugin switched on there that isn't the base game, `_ResourcePack.esl`, listed in `Skyrim.ccc` (Creation Club) or shipped by the server is listed, and so is a broken plugin file (bad header, version 0, no records). **Fix it** switches them off in plugins.txt and keeps a copy of the old file as `plugins.txt.aetherial-dawn-backup`. The plugin files stay in Data. Vortex rewrites plugins.txt when it deploys, so Vortex users also switch the plugin off in Vortex's Plugins tab. In the first live test, a 59-byte `SkyUI_SE.esp` stub from an old setup was the only active plugin.

## Tidying before every Play

Players don't click anything for this. Before each launch, the launcher:
- moves SKSE/Platform plugins and loose `Data/Interface` files the server didn't ship (old RaceMenu, map and HUD menus) to `.aetherial-dawn/disabled/<time>/`;
- switches extra or broken plugins off in plugins.txt;
- drops archives from `sResourceArchiveList`/`sResourceArchiveList2` in Skyrim.ini, SkyrimPrefs.ini and SkyrimCustom.ini that no longer exist in Data, plus repeated ones. The original ini is kept once as `<name>.aetherial-dawn-backup`.

Everything it changes is written to the log. In the first live test, Skyrim.ini still named 43 BSAs from uninstalled mods.

## Plugins for a newer Skyrim

Steam updates Creation Club downloads separately from the game, so after a downgrade they can stay on the newer build. Before every Play, `loadorder::too_new` compares each plugin's header version and form version with the five base masters. Any plugin that is newer, and any broken stub plugin, is moved with its archives to `.aetherial-dawn/disabled/<time>-plugins/`. The game skips Creation Club files that aren't in Data. The version record and the kept copy are then refreshed. Skyrim Platform's browser cache (`%TEMP%\\Skyrim Platform`) is cleared, because a stale profile there crashed libcef.dll 5 seconds in. A crash logger that launchers before 0.1.20 moved aside is put back, so the next crash names the failing module. Crash reports to staff go out in the background and are retried once when the staff service asks for a short wait.

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

The launcher installs new releases by itself, without asking. It checks at startup and every 15 minutes, but never while Skyrim is running or a download is in progress.

## Keeping the game on the right build

After a downgrade (either way), the launcher sets `"AutoUpdateBehavior" "1"` (only update when launched) in `steamapps/appmanifest_489830.acf` and makes that file read-only, so Steam doesn't swap the game data back to the newer build. The game is always started through SKSE, never through Steam. Server Info always offers **Fix version** / **Re-download** when the server lists depots. **My game is already on this version** needs a second click, and the mark it leaves is removed automatically if Skyrim then crashes, so the version check runs again. In the first live test, a player used it over updated game data and Skyrim crashed with an access violation after 4 seconds.

## Reporting problems

The launcher keeps a log at `%LOCALAPPDATA%\gg.aetherialdawn.launcher\logs\launcher.log` (the previous one is kept as `launcher.old.log` once it passes 2 MB). It records startup, every command and how it ended, sign-in results, the downgrade steps and DepotDownloader arguments, and script errors. It never records the Discord token, passwords or game sessions. **Settings, Copy diagnostics** puts a report on the clipboard: launcher version, Skyrim exe version, SKSE files, Steam depots, the version check, server build, sign-in state and the last 80 log lines. **Open log folder** opens the log folder.

## Known gaps

- **Discord sign-in:** built to aetherial-dawn-discord/CONTRACT.md and tested against a stand-in service, not yet the live one. The file name of the game's remembered login (`auth-data-no-load.js`) is inferred from the SkyMP client source and needs checking on the first real test.
- **Downgrader:** tested end to end with a stand-in for DepotDownloader, not yet against real Steam. Players on non-Steam copies can't use it.
- **Game detection:** only Steam installs are found automatically. GOG and other installs use the folder picker.

Fonts are Cinzel, Hanken Grotesk and JetBrains Mono, all under the SIL Open Font License.
