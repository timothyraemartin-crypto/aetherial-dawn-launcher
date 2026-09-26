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
- `app/latest.json` and the installer, used for launcher self-updates.
- `status.json` (optional), which feeds the side panel: `{ "online": true, "players": 7, "maxPlayers": 100, "sinceReset": "1d", "news": [{ "date": "26 Sep 2026", "title": "…", "body": "…" }] }`

## Building

Windows installers are built by `.github/workflows/build.yml`. Before a real release:

1. Set the repository variable `AD_BASE_URL` to the server's launcher URL.
2. Create an update signing key on your own PC by running `cargo tauri signer generate -w ~/.tauri/aetherial.key`. Put the private key and its password in the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Never paste them into chat.
3. Put the public key in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`. Point `endpoints` at `<AD_BASE_URL>/app/latest.json`.
4. Replace the placeholder icons in `src-tauri/icons/`, then run `cargo tauri icon your-logo.png`.

To build locally on Windows, run `cargo install tauri-cli --version "^2"`, then `cargo tauri build`.

To cross-build the Windows installer from Linux, as used for the first test build, install `gcc-mingw-w64-x86-64`, `nsis` and `cargo install tauri-cli`. Then run `rustup target add x86_64-pc-windows-gnu` and `AD_BASE_URL=<url> cargo tauri build --target x86_64-pc-windows-gnu --bundles nsis --config '{"bundle":{"createUpdaterArtifacts":false}}'`. Tauri calls this cross-build experimental, and the installer isn't code-signed.

To test the file sync against any server, run `cargo run -p launcher-core --example sync -- <base-url> <skyrim-folder>`.

## Game version check and downgrader

The server's `manifest.json` names the Skyrim build it needs (`game.version`, now 1.6.1170.0 with SKSE 2.2.6) and the Steam depot manifests for it. `core/src/version.rs` compares the player's SkyrimSE.exe, Steam's `appmanifest_489830.acf` and the launcher's own record. `core/src/downgrade.rs` fixes a mismatch by running DepotDownloader in its own window, where the player signs in with their own Steam account. The full format is in the spec.

To test without Steam, point `game.tool.url` at a zip holding a stand-in `DepotDownloader`. `cargo run -p launcher-core --example gamever -- SkyrimSE.exe` prints an exe's version.

## Known gaps

- **Player identity:** the launcher sends a random `profileId` saved on the player's PC. SkyMP's offline mode trusts that number, so players aren't truly authenticated yet. The plan is Discord sign-in checked by the server.
- **Downgrader:** tested end to end with a stand-in for DepotDownloader, not yet against real Steam. Players on non-Steam copies can't use it.
- **Game detection:** only Steam installs are found automatically. GOG and other installs use the folder picker.

Fonts are Cinzel, Hanken Grotesk and JetBrains Mono, all under the SIL Open Font License.
