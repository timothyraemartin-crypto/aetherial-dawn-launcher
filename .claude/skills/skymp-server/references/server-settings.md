# server-settings.json reference

Official rule: "The recommended way to configure the server is setting up all
required values in `server-settings.json`. It's standard JSON without C-style
comments support." `[V]` (docs/docs_server_configuration_reference.md,
mirrored at pospelovlm.gitbook.io/skyrim-multiplayer-docs)

Validate the user's file as strict JSON before anything else. A single `//`
comment or trailing comma explains a lot of "settings ignored" reports.

## Verified keys `[V]`

| Key | Type | Example | Meaning |
|---|---|---|---|
| `name` | string | `"My Server"` | Shown on skymp.io and in the launcher |
| `ip` | string | `"127.0.0.1"` | Address clients connect to. Docs: "Do not try to type "0.0.0.0", just remove this option from document if you want to use your current public IP." |
| `port` | number | `7777` | Main UDP port |
| `maxPlayers` | number | `108` | Player cap shown in launcher/skymp.io |
| `dataDir` | string | `"data"` | Relative path to the folder with esp/esm/pex |
| `loadOrder` | string[] | the five vanilla masters | "A list of esp/esm files which would be loaded by the server during startup in the same order as Skyrim SE loads them" |
| `databaseDriver` | string | `"file"` | `file` / `zip` / `mongodb` / `migration` (config reference also says `sqlite`; see linux-setup.md §6) |
| `databaseName` | string | `"world"` | Folder/zip name, or Mongo DB name |
| `databaseUri` | string | `"mongodb+srv://..."` | MongoDB only |
| `databaseOld` / `databaseNew` | object | nested driver configs | `migration` driver only |
| `reloot` | object | `{"CONT": 86400000}` | Record type → ms before reset. Types seen: FLOR, TREE, AMMO, ARMO, BOOK, INGR, ALCH, SCRL, CONT, SLGM, WEAP, MISC. Default behaviour resets everything hourly. |
| `gamemodePath` | string | `"gamemode.js"` | File, or directory containing `index.js` |
| `isPapyrusHotReloadEnabled` | bool | `false` | Hot-reload `.pex`; costs performance, keep off in production |
| `locale` | string | `"ru-RU"` | File in `data/localization` (no extension) used by `M.GetText` |

## Secondary-source keys `[S]` — confirm in `skymp5-server/ts/settings.ts`

| Key | Type | Meaning | Source |
|---|---|---|---|
| `masterKey` | string | Key for Master API authentication | DeepWiki (settings.ts:16-76) |
| `offlineMode` | bool | No auth; `profileId` becomes client-controlled. CMake `OFFLINE_MODE` defaults ON in generated settings. Forks: "must not be trusted for a public production server" | DeepWiki + fork READMEs |
| `npcEnabled` | bool | Enable/disable NPC sync | DeepWiki |
| `uiPort` | number | HTTP UI port (default 3000, ui.ts:102) | DeepWiki |

## Not found anywhere `[U]` — do not use without reading settings.ts

`master`, `startPoints`, `spawnPoint`, `lang`, `archives`, `allSettings`,
`additionalServerSettings`. DeepWiki says settings "can be extended with
additional settings fetched from GitHub repositories" (settings.ts:16-76);
the key names and any environment variables for this are `[U]`.

## Command-line overrides `[V]`

```
skymp5-server --maxPlayers 108 --name "Server X" --port 7777 --ip "127.0.0.1"
```
"The only options that exist as command-line arguments are maxPlayers, name,
port, ip." Only `maxPlayers` has a short form (`-m`). CLI arguments
"implicitly override values from server-settings.json".

## Ports `[V]`

| Port | Default | Rule |
|---|---|---|
| Main UDP | 7777 | `port` |
| UI HTTP | 3000 | main+1 if main is non-default |
| WebSocket | 8080 | main+2 if main is non-default |
| Webpack dev | 1234 | dev only |
| Chromium DevTools | 9000 | client machine, `localhost:9000` |

## Annotated example (strict JSON, notes below)

```json
{
  "name": "My Linux SkyMP Server",
  "port": 7777,
  "maxPlayers": 50,
  "dataDir": "data",
  "loadOrder": [
    "Skyrim.esm",
    "Update.esm",
    "Dawnguard.esm",
    "HearthFires.esm",
    "Dragonborn.esm",
    "MyServerItems.esp"
  ],
  "databaseDriver": "file",
  "databaseName": "world",
  "gamemodePath": "gamemode.js",
  "isPapyrusHotReloadEnabled": false,
  "offlineMode": false,
  "reloot": { "CONT": 86400000, "FLOR": 86400000 }
}
```

- `ip` omitted → public IP used.
- `offlineMode` is `[S]`; keep it but confirm the key exists in your
  `settings.ts`. If the server logs an unknown-key warning, remove it and
  check how your build toggles auth.
- Vanilla masters first, in exactly this order, then your plugins in the
  clients' `plugins.txt` order.

## Review checklist for a pasted settings file

1. Parses as strict JSON (no comments, no trailing commas).
2. `ip` is absent or a real routable address, never `0.0.0.0`.
3. `loadOrder` starts with the five masters; no `.esl`; names match files in
   `dataDir` case-exactly; every plugin's masters appear earlier.
4. `databaseDriver` present and explicit.
5. `isPapyrusHotReloadEnabled` false for production.
6. `offlineMode` false for anything public.
7. Every key not in the verified or secondary tables gets flagged as `[U]`.
