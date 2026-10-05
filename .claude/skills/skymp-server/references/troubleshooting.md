# Troubleshooting

Ask for: the last 50 lines of server output, `server-settings.json`, the
client's `plugins.txt`, the SP log, and the `SkyrimSE.exe` version. Then use
this table. Quoted strings are exact where marked `[V]`.

## Startup and build

| Symptom | Likely cause | Fix | Status |
|---|---|---|---|
| Server exits/hangs during load; last line `[ESPM] X read in ...` | Plugin X is ESL/ESPFE, has a missing master, or a malformed record | `scripts/scan_plugins.py`, then `mod-compatibility.md` §4 | `[V]` log format, `[I]` cause |
| "Settings ignored" / JSON parse error | Comments or trailing commas in `server-settings.json` | Strict JSON | `[V]` |
| `Unknown bind__() error 98` | errno 98 = EADDRINUSE: port already bound (another instance, or a test run) | `ss -lunp \| grep 7777`, kill it or change `port` | `[V]` issue #111 |
| Native addon fails to load / ABI error | Node major mismatch between build and run | Use the same Node major (22.x per Dockerfile) | `[I]` |
| CMake: vcpkg toolchain not found | submodules missing | `git submodule init && git submodule update` | `[V]` |
| Build errors mentioning GCC / libstdc++ | GCC used | clang (15 or 20) | `[V]` |
| Fails on Alpine or Arch | unsupported distro | Ubuntu or the official Docker image | `[V]` |
| Papyrus-heavy server after hot reload is slow | `isPapyrusHotReloadEnabled: true` | set false | `[V]` |

## Connectivity and UI

| Symptom | Likely cause | Fix | Status |
|---|---|---|---|
| Internet players can't see/join; LAN works | `ip` set to `127.0.0.1`/`0.0.0.0`, or UDP port closed | Remove `ip` (public IP) or set the routable one; open UDP 7777 | `[V]` |
| Players join but in-game UI is blank | UI (3000) or WebSocket (8080) blocked, or `data/ui/index.html` missing | Open TCP 3000/8080 (main+1/main+2); build skymp5-front into `data/ui` | `[V]` |
| Server missing from skymp.io / launcher list | `offlineMode` on, no/invalid `masterKey`, master unreachable | Master API auth; confirm keys in `settings.ts` | `[S]` |
| Admin/ban bypass, spoofed names | `offlineMode: true` publicly | Turn off; `profileId` is client-controlled in offline mode | `[S]` |

## Client

| Symptom | Likely cause | Fix | Status |
|---|---|---|---|
| SKSE: "expecting an SKSE for runtime version 1.7.99/1.7.104" or similar | Steam auto-updated | Downgrade to 1.6.1170, SKSE 2.2.8 (`versions.md`) | `[V]` pattern |
| "...failed to locate an appropriate address library..." | DLL/runtime mismatch | Address Library AE; remove outdated DLLs from `Data/SKSE/Plugins` | `[V]` |
| Crash on launch with SKSE | an outdated SKSE DLL | SKSE's advice: "Remove everything from Data/SKSE/Plugins", add back one by one | `[V]` |
| Chromium process lingers after exit; UI hangs | Engine Fixes missing | Install SSE Engine Fixes | `[V]` SP README |
| Save won't load after SP update | SP changed save state | New game/profile | `[V]` SP README |
| Linux client crashes on SP startup under Proton | known | "some crashes can occur on SP startup"; retry, keep Proton version stable | `[V]` |

## In-game behaviour

| Symptom | Likely cause | Fix | Status |
|---|---|---|---|
| Wrong items, invisible objects, "bad form"-type errors | Load order or plugin version differs client vs server | `scripts/check_loadorder.py`; redistribute pack; masters from 1.6.1170 | `[V]` rule, `[U]` exact strings |
| Containers never refill / refill too fast | `reloot` defaults (hourly) | Tune `reloot` per record type | `[V]` |
| Mod loads but its feature doesn't work or differs per player | Script/quest/AI driven, not server-simulated | Reimplement server-side or drop | `[I]` |
| Server crash after activating an object; log has "Method not found" / "Out of stack space" | Server-side Papyrus hit an unimplemented native or recursed | Remove the `.pex` or implement the native (functions-lib) | `[S]` |
| Remote players T-pose or play wrong animations | Behavior files differ between players | Same animation framework output on every client | `[I]` |

## Log strings worth grepping

- `[ESPM]` — plugin load progress; the last one before a crash is the culprit.
- `Method not found` / `Out of stack space` — Papyrus VM.
- `bind__` / `error 98` — port in use.
- `manifest` — parity-related messages.
- SP client log: `Documents\My Games\Skyrim Special Edition\SKSE\`.

## Unknowns to settle when you have the repo

Exact client-side load-order mismatch message; whether the client hard-fails
on crc32 mismatch; the Linux Papyrus compile path; the production start
script. See `sources.md`.
