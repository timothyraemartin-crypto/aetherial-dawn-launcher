# SkyMP architecture

Mental model: the server builds the world from the plugin files you give it,
holds the authoritative copy of game state, and tells clients what to render.
Clients render from their own identical copy of the plugins. Skyrim's own
simulation (quests, AI, most scripts) does not run on the server.

## Repository layout (skyrim-multiplayer/skymp, monorepo) `[V]` unless marked

- `skymp5-server/` — the server.
  - `ts/` Node orchestration: `index.ts`, `settings.ts`, `ui.ts` (Koa HTTP
    server + Prometheus metrics via prom-client), `systems/` (login.ts,
    spawn.ts, masterClient.ts implementing `initAsync`, `connect`,
    `disconnect`, `customPacket`) `[S]`.
  - `cpp/` native addon: `addon/ScampServer.cpp` (the JS↔C++ bridge),
    `server_guest_lib/` (`PartOne` game loop, `WorldState`, `MpActor`,
    `MpObjectReference`, `MpChangeForms.h`, `database_drivers/`,
    `script_classes/` for Papyrus natives), `mp_common/` (`Networking.cpp`,
    message archives).
- `skymp5-client/` — TypeScript client running inside SkyrimPlatform.
  `src/services/services/remoteServer.ts` handles server messages `[S]`.
- `skymp5-front/` — React in-game UI, served by the server.
- `skymp5-functions-lib/` — Papyrus natives implemented in TypeScript
  (vanilla/SKSE mimics plus `M.*` helpers like `M.GetPlayersOnline`,
  `M.GetText`).
- `skymp5-scripts/` — Papyrus `.psc`/`.pex` used by the reference gamemode.
- `skyrim-platform/` — SP source (`SkyrimPlatform.dll`, `SkyrimPlatformImpl.dll`).
- `papyrus-vm/` — standalone Papyrus VM reimplementation.
- `libespm/` — esp/esm parser (`espm.h`, `Combiner.h`, `BrowserInfo.cpp`).
- `savefile/`, `serialization/`, `viet/`, `unit/`, `client-deps/`,
  `overlay_ports/`, `overlay_triplets/`, `vcpkg` submodule.
- Root: `Dockerfile`, `build.sh`, `CMakeLists.txt`, `vcpkg.json`,
  `ROADMAP.md`, `TERMS.md` (requires disclosing fork source), `CLAUDE.md`,
  `CONTRIBUTING.md`, `misc/` (`github_env_linux`, tests).

## Components at runtime

| Piece | Runs where | Language | Role |
|---|---|---|---|
| skymp5-server | your Linux host | Node + C++ addon | world state, networking, Papyrus VM, DB |
| skymp5-client | inside SkyrimSE.exe | TS on SkyrimPlatform | applies server state to the local game, sends local actions |
| SkyrimPlatform | inside SkyrimSE.exe | SKSE plugin (C++) | JS runtime + Chromium (CEF) UI host |
| skymp5-front | browser inside the game | React | in-game UI, talks to server over WebSocket |
| Master API / skymp.io | Anthropic-external, SkyMP's | — | server listing + identity (`profileId`) unless `offlineMode` |

## World model and FormIDs

- On startup the server reads every `loadOrder` entry from `dataDir` and
  logs `[ESPM] Skyrim.esm read in 0.10s, parsed in 3.09s, size is 238Mb`
  style lines. The last such line before a crash identifies the bad plugin.
  `[V]` (log format seen in official docs/issues)
- Load-order index decides the FormID high byte. That is why server and
  client order must match exactly. `[V]`
- The server writes `data/manifest.json`: `versionMajor`, a `mods` array of
  `{crc32, filename, size}`, and `loadOrder`. "Do not modify that file." `[V]`
- Runtime objects (players, spawned refs) get FormIDs at `0xFF000000` and up.
  The docs' examples use `0xff000000` for the first player. `[V]`/`[S]`
- `FormDesc` stores FormID + plugin filename so persisted references survive
  load-order changes in principle (`MpChangeForms.h`). `[S]` Do not rely on
  this; back up before reordering.
- ESL/light plugins: not supported (issue #530, closed as "temporarily
  unsupported"). How libespm treats the 0x200 flag and 0xFE indices today is
  `[U]`; the safe assumption is "breaks or misloads". `[V]`+`[I]`
- BSAs are client-side only. "currently .bsa archives are used only on the
  client-side." Name them after the plugin (`FooBar.bsa` for `FooBar.esp`). `[V]`

## Sync model

- README: "Mostly server-controlled game state - you can't cheat everything". `[V]`
- Done per ROADMAP: appearance (race, headparts, tints), attributes
  (health/magicka/stamina), inventory. Movement: "working sync, requires lag
  compensation". `[V]`
- Server-authoritative: inventory, containers, actor values, properties. `[S]`
- Client-originated: movement, animation events, equipment changes, hits,
  spell casts; the server validates/forwards. `[I]`
- Client message handlers in `remoteServer.ts` `[S]`: `CreateActorMessage`
  (spawn via `moveRefrToPosition` or `LoadGameService.loadGame()`, applies
  `customPropsJsonDumps`), `UpdateMovementMessage`, `UpdateAnimationMessage`,
  `UpdateEquipmentMessage`, `UpdatePropertyMessage` (persistent refs below
  `0xff000000` applied directly), `OpenContainerMessage` (activate, wait for
  menu close, second activation to server), `SpellCastMessage`.
- Client keeps `WorldModel` in `storage['worldModel']` so it survives SP hot
  reload. `[S]`
- No vanilla cell reset. "The server resets every object in the world every
  hour instead", tunable per record type via `reloot`. `[V]`
- NPC sync can be toggled with `npcEnabled`. `[S]`
- Players get dynamic FormIDs; `mp.get(0xff000000, "pos")` reads the first
  player's position. `[V]`

## Networking

| Port | Protocol | Default | Rule `[V]` |
|---|---|---|---|
| Main | UDP | 7777 | `port` / `--port` |
| UI | HTTP(S) | 3000 | "Equals (Main Port + 1) if its value is non-default" |
| WebSocket | WS | 8080 | "Equals (Main Port + 2) if its value is non-default" |
| Webpack dev server | HTTP | 1234 | dev only; server proxies UI to it if running |
| Chromium DevTools | — | 9000 | client-side; open `localhost:9000` in a real browser |

- Transport code: `skymp5-server/cpp/mp_common/Networking.cpp`, serialization
  via `BitStreamInputArchive`/`JsonInputArchive`. "BitStream" points at a
  RakNet/SLikeNet lineage. Exact library fork, tick rate, packet limits and
  rate limiting are `[U]`. `[S]`+`[I]`
- Firewall: open UDP 7777 **and** TCP 3000 + 8080 (or main+1/main+2).

## Persistence

Drivers (`database_drivers/`): File, Zip, Mongo, Migration. No SQLite driver
appears in the source listing despite one doc saying "sqlite by default". `[S]`
See `linux-setup.md` → Persistence for config and backups.

## Server-side scripting surfaces

1. **TypeScript/JS gamemode** at `gamemodePath` (a file, or a directory with
   `index.js`) using the global `mp` object. See `scripting-api.md`.
2. **Server-side Papyrus**: `.pex` in `data/scripts`, run on the custom VM,
   natives from `script_classes/` + skymp5-functions-lib. Hot reload via
   `isPapyrusHotReloadEnabled`. See `scripting-api.md`.

## Authentication

- With `offlineMode: false` the server resolves identity via the Master API
  (`masterKey`). With `offlineMode: true`, `profileId` is client-controlled
  and "must not be trusted for a public production server, staff
  authorization or bans" (fork README). `[S]`
- The CMake option `OFFLINE_MODE` defaults to ON in generated settings, so a
  fresh build is in offline mode until you change it. `[S]`
