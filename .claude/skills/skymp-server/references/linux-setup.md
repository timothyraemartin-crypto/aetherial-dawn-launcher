# Linux server setup runbook

## Contents
1. Toolchain facts (from the official Dockerfile)
2. Path A — build inside the official Docker build image
3. Path B — bare metal from source
4. Dist layout and start command
5. Server data directory
6. Persistence: drivers, config, backups
7. systemd, firewall, updates
8. Player-side install checklist
9. Known build failures

## 1. Toolchain facts `[V]` from root `Dockerfile` (main, Oct 2026)

- `skymp-runtime-base`: `FROM ubuntu:25.10`, Node.js **22.x** via NodeSource
  (`curl -fsSL https://deb.nodesource.com/setup_22.x | bash -`), plus `gdb`,
  user `skymp` created with `useradd -m skymp`.
- `skymp-build-base`: yarn, libicu-dev, git, CMake from apt.kitware.com
  (`jammy` repo), curl, unzip, tar, make, zip, pkg-config, flex, bison,
  autoconf, autoconf-archive, automake, libtool, **clang-20**,
  clang-format-20, ninja-build, build-essential. `clang`/`clang++` symlinks
  point at the `-20` binaries.
- `skymp-vcpkg-deps-builder`: clones vcpkg at `VCPKG_COMMIT`, runs
  `./build.sh --configure`. The vcpkg cache lives at
  `/home/skymp/.cache/vcpkg` ("the builtin NuGet cache doesn't work on Linux").
- Published images on Docker Hub (namespace `skymp`): `skymp-vcpkg-deps`
  and `skymp-runtime-base` updated ~6 months ago; `skymp-server`,
  `skymp`, `skymp-base` are 3–5 years stale. **Build your own runtime
  image; do not pull `skymp/skymp-server`.** `[V]`
- Older guidance (CONTRIBUTING.md): Ubuntu 22.04 + clang-15, "GCC is not
  supported", Alpine "doesn't work", Arch "won't be able to run the server"
  (ChakraCore issue), ~4 GB RAM and ~22 GB disk for a build. `[V]` The
  Dockerfile is newer; follow it, but the "no GCC / no Alpine / no Arch"
  rules still hold. `[I]`

## 2. Path A — official Docker build image (recommended)

```bash
git clone https://github.com/skyrim-multiplayer/skymp.git
cd skymp
git submodule init && git submodule update
. misc/github_env_linux            # exports SKYMP_VCPKG_DEPS_IMAGE
docker run -it --rm -v "$PWD:$PWD" -w "$PWD" -u "$(id -u):$(id -g)" \
    "$SKYMP_VCPKG_DEPS_IMAGE" bash
# inside the container:
./build.sh --configure -DCMAKE_BUILD_TYPE=Release
cd build && ../build.sh --build --parallel "$(nproc)"
```
`[V]` from CONTRIBUTING.md. Rootless Podman works; on SELinux hosts add
`--security-opt label=disable` and
`-e VCPKG_DEFAULT_BINARY_CACHE=/home/skymp/.cache/vcpkg/archives`. `[V]`

Useful CMake options `[S]` (DeepWiki reading of CMakeLists.txt):

| Option | Default | Note |
|---|---|---|
| `BUILD_UNIT_TESTS` | ON | turn OFF for a faster server-only build |
| `BUILD_CLIENT` | ON | client needs MSVC; OFF on Linux |
| `BUILD_GAMEMODE` | OFF | |
| `BUILD_FRONT` | OFF | ON if you want `skymp5-front` built into `data/ui` |
| `SKYRIM_VR` | OFF | |
| `OFFLINE_MODE` | ON | generated `server-settings.json` gets offlineMode on |
| `SKYRIM_DIR` | — | path to a Skyrim install; enables ESPM tests and Papyrus compile |
| `UNIT_DATA_DIR` | — | |

Without `SKYRIM_DIR`, "the server would require manual installation of
Skyrim.esm and other master files." `[V]`

## 3. Path B — bare metal from source

Mirror the Dockerfile: Ubuntu (22.04 or newer), clang (15 or 20), CMake ≥ 3.19
from Kitware, Node 22 (nvm or NodeSource), yarn, ninja, and the autotools
list above. Then the same `./build.sh --configure` / `--build` commands.
Run tests with `ctest -C Debug --verbose`. ESPM tests require unmodified
vanilla masters. `[V]`

The client "can only be built using MSVC". Build it on Windows or use a
release artifact. Linux players can run the client under Proton; "some
crashes can occur on SP startup." `[V]`

## 4. Dist layout and start command

- Output: `build/dist/` with `server/`, `client/`, `papyrus/` folders. `[S]`
- `build/dist/server/` contains `scam_native.node`, `skymp5-server.js`, a
  generated `server-settings.json` (from
  `cmake/scripts/generate_server_settings.cmake`) and a `README.md` with a
  settings template. `[S]`
- Start: `cd build/dist/server && node skymp5-server.js` as an unprivileged
  user. `[I]` The exact `package.json` start script is `[U]`; community
  builds (Red House) use `npm start` in the server folder or
  `npm run server:start` at repo root. `[V]` Check `package.json` in your dist.
- Logs go to stdout/stderr. Under systemd read them with
  `journalctl -u skymp -f`.

## 5. Server data directory `[V]` (docs_server_data_directory)

```
server/
  server-settings.json
  gamemode.js                 (or gamemode/index.js)
  world/                      (file DB, created at runtime)
  data/
    Skyrim.esm Update.esm Dawnguard.esm HearthFires.esm Dragonborn.esm
    YourMod.esp               (every plugin in loadOrder, case-exact names)
    scripts/                  (.pex for server-side Papyrus; source/ for .psc)
    ui/index.html             (bundled front-end)
    localization/ru-RU.json   (for "locale")
    manifest.json             (generated at startup: do not edit)
    _libkey.js                (embedded into the CEF page: do not edit)
```

- You do not need Skyrim installed on the server, only the master and plugin
  files, copied from a **1.6.1170** install.
- BSAs are not needed on the server ("used only on the client-side") but
  must exist on every client, named after their plugin.
- Linux is case-sensitive: `MyMod.esp` ≠ `mymod.esp`.
- The service user needs write access to `world/` and `data/` (manifest).

## 6. Persistence: drivers, config, backups

Drivers `[V]` (docs_database_drivers.md):

| Driver | Config | Notes |
|---|---|---|
| `file` | `"databaseName": "world"` | Directory under the server folder. "Only relative paths are supported." Docs say this is the default. |
| `zip` | `"databaseName": "world"` | Same as file but a `world.zip` archive. |
| `mongodb` | `"databaseName"`, `"databaseUri"` | "Built for servers targeting real-world players from the Internet." Docs recommend Atlas. |
| `migration` | `"databaseOld": {...}, "databaseNew": {...}` | Copies on the fly. "Do not forget to backup everything before using this." |

The configuration reference says "`sqlite` by default" while the drivers doc
says `file` is the default, and no SQLite driver appears in the source
listing. **Always set `databaseDriver` explicitly.** `[V]`+`[S]`

Migration example `[V]`:
```json
{
  "databaseDriver": "migration",
  "databaseOld": { "databaseDriver": "file", "databaseName": "world" },
  "databaseNew": { "databaseDriver": "mongodb", "databaseName": "skymp", "databaseUri": "mongodb+srv://..." }
}
```
Run once, verify, then switch `databaseDriver` to the new driver.

Backups `[I]`:
```bash
# file driver
systemctl stop skymp && tar czf "world-$(date +%F).tgz" world/ && systemctl start skymp
# mongodb
mongodump --uri "$URI" --out "backup-$(date +%F)"
```
Also back up `server-settings.json`, the gamemode and `data/` before every
dist update. A community guide's warning, verbatim: "BACK UP YOUR
GAMEMODE.JS AND YOUR SERVER-SETTINGS.JSON!!!!!!" `[V]`

Changing `loadOrder` on an existing world can shift FormIDs in saved
references. Test on a staging copy first. `[I]`

## 7. systemd, firewall, updates

`assets/skymp.service` is a template. Key points: run as `skymp`, set
`WorkingDirectory` to the dist server folder, `Restart=on-failure`,
`LimitNOFILE=65536`. Adjust `ExecStart` to your dist's real entry point.

```bash
sudo ufw allow 7777/udp
sudo ufw allow 3000/tcp
sudo ufw allow 8080/tcp     # or port+1 / port+2 if port is non-default
```

Update procedure: stop → back up (section 6) → replace dist → restore
`server-settings.json`, gamemode, `data/` plugins → start → watch the
`[ESPM]` lines → connect one client before announcing.

## 8. Player-side install checklist (put this in your server docs)

1. Skyrim SE/AE **1.6.1170**, Steam auto-update off (see `versions.md`).
2. SKSE64 2.2.8, Address Library (AE), SSE Engine Fixes (both parts).
3. SkyrimPlatform 2.9.0 → `Data/Platform/Plugins/`, settings in
   `Data/Platform/Plugins/<name>-settings.txt`, ini at
   `Data/SKSE/Plugins/SkyrimPlatform.ini`.
4. The SkyMP client.
5. The server's exact plugin pack: same files, same order, same versions,
   with matching BSAs. Ship it as one versioned archive or MO2 profile with
   a `SHA256SUMS` file. No FOMOD choices that change plugins.
6. New save/profile. SP: "Updating/deleting SkyrimPlatform on a current save
   might break your save."
7. Verify `SkyrimSE.exe` version before reporting any bug.

## 9. Known build failures

| Symptom | Cause | Fix |
|---|---|---|
| CMake can't find the vcpkg toolchain | submodules not initialised | `git submodule init && git submodule update` `[V]` |
| Build fails with GCC errors | unsupported compiler | use clang; the Dockerfile uses clang-20 `[V]` |
| Fails on Alpine / Arch | unsupported distro (ChakraCore) | Ubuntu or the Docker image `[V]` |
| vcpkg rebuilds everything every run | cache not mounted | mount `/home/skymp/.cache/vcpkg` or set `VCPKG_DEFAULT_BINARY_CACHE` `[V]` |
| Node native addon load error at runtime | Node major mismatch between build and run | run with the same Node major used to build (22.x per Dockerfile) `[I]` |
| ESPM unit tests fail | modified or missing masters | pure vanilla masters, `SKYRIM_DIR` set `[V]` |
