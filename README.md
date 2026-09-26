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

## Downgrading through the Steam app

The recommended downgrade option uses the Steam app the player is already signed into. Steam doesn't let other programs start a depot download, so the launcher opens Steam's console (`steam://open/console`) and shows the three `download_depot 489830 <depot> <manifest>` lines with Copy buttons. Steam downloads each depot into `<Steam>/steamapps/content/app_489830/depot_<id>/`. The launcher watches those folders, and when all of them have been quiet for 10 seconds and the player clicks Install, it copies the files into the game folder, deletes the downloaded copy, and checks the version. The DepotDownloader options (Steam mobile app QR, or account name) stay as a fallback for when Steam isn't running.

## Crash reports

After Play the launcher hides instead of closing and watches `SkyrimSE.exe`. When the game ends it saves `game-<time>.txt` next to the launcher log: how long Skyrim ran, its exit code in plain words, every log written that session under `Documents/My Games/Skyrim Special Edition/SKSE` (including crash logger output) and SkyrimPlatform's temp folder, and the last 40 launcher log lines. A crash (a Windows exception code, or the game closing within 90 seconds) brings the launcher back with the report and a Copy report button; a normal quit closes the launcher. Copy diagnostics includes the latest report.

## Plugins from other mods

After each check, the launcher lists every `.dll` in `Data/SKSE/Plugins` and every file in `Data/Platform/Plugins` that isn't in the server's file list (the SkyMP settings file aside). Leftover plugins like these crashed the game before the main menu in the first live test. Play turns into **Check mods**, which shows the list and moves the files to `.aetherial-dawn/disabled/<time>/` inside the game folder, keeping their paths. Nothing is deleted. Players can also choose to play with them anyway, which is logged.

## Reporting problems

The launcher keeps a log at `%LOCALAPPDATA%\gg.aetherialdawn.launcher\logs\launcher.log` (the previous one is kept as `launcher.old.log` once it passes 2 MB). It records startup, every command and how it ended, sign-in results, the downgrade steps and DepotDownloader arguments, and script errors. It never records the Discord token, passwords or game sessions. **Settings, Copy diagnostics** puts a report on the clipboard: launcher version, Skyrim exe version, SKSE files, Steam depots, the version check, server build, sign-in state and the last 80 log lines. **Open log folder** opens the log folder.

## Known gaps

- **Discord sign-in:** built to aetherial-dawn-discord/CONTRACT.md and tested against a stand-in service, not yet the live one. The file name of the game's remembered login (`auth-data-no-load.js`) is inferred from the SkyMP client source and needs checking on the first real test.
- **Downgrader:** tested end to end with a stand-in for DepotDownloader, not yet against real Steam. Players on non-Steam copies can't use it.
- **Game detection:** only Steam installs are found automatically. GOG and other installs use the folder picker.

Fonts are Cinzel, Hanken Grotesk and JetBrains Mono, all under the SIL Open Font License.
