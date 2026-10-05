# SkyrimPlatform (SP)

SP is an SKSE plugin that embeds a JavaScript runtime and Chromium (CEF). The
SkyMP client "is technically a mod for Skyrim Special Edition implemented
using Skyrim Platform." `[V]` SP ≠ SKSE: SKSE is still required at runtime,
and SKSE DLL plugins are separate from SP plugins.

## Supported runtimes `[V]` (`skyrim-platform/README.md`)

- "Steam latest (currently Skyrim SE/AE 1.6.1170)"
- "Skyrim SE/AE 1.6.640"
- "Skyrim SE 1.5.97.0.8 (Deprecated, not accepting bugs anymore)"
- 1.6.1130 and 1.7.x are **not** listed.

## Releases

| Version | Highlights `[V]` from `docs/release/sp-*.md` |
|---|---|
| 2.9.0 (current, Nexus 54909) | "Added support for Skyrim 1.6.1170.0". "all remaining unsafe event handlers have been resolved. Crashes when loading a save should no longer occur." New natives `TESModPlatform.CreateReferenceAtLocation`, `TESModPlatform.CloseMenu`; `blockPapyrusEvents` alias; `loadGame` gained `time` and `loadOrder` params; `Game.getModCount`/`getModName` work in tick context; `setInventory` no longer denies unequipping for the player. "New game required." |
| 2.7.0 | Added 1.6.640. Calling a non-native Papyrus function now throws a JS exception instead of crashing. |
| 2.6.0 | Added AE/1.6 support, Address Library IDs instead of offsets, introduced `SkyrimPlatform.ini`. Log goes to `Documents\My Games\Skyrim Special Edition\SKSE`. |

The SkyMP client requires SP 2.9.0 (`skymp5-client/src/version.ts`). `[S]`

## Install layout `[V]`

- Plugins: `Data/Platform/Plugins/*.js`
- Per-plugin settings: `Data/Platform/Plugins/<name>-settings.txt`
- ini: `Data/SKSE/Plugins/SkyrimPlatform.ini`
- Requires SKSE64 (2.2.8 for 1.6.1170), Address Library (AE). SP README:
  use SSE Engine Fixes, otherwise the "Chromium process will hang".
- "Updating/deleting SkyrimPlatform on a current save might break your
  save." Start a new game after SP changes.

## `SkyrimPlatform.ini` `[V]` (`docs/skyrim_platform/ini_settings.md`)

```ini
[Debug]
LogLevel=2          ; 0 trace ... 6 none
Cmd=false           ; in-game console window
CmdOffsetLeft=0
CmdOffsetTop=720
CmdWidth=1900
CmdHeight=317

[Browser]
BackendName=auto    ; off | tilted (legacy) | nirnlab (NirnLab UI Platform) | auto
                    ; "auto" currently falls back to tilted
```

## Behaviour notes

- Hot reload: SP watches plugin files; a changed `.js` reloads all plugins.
  The SkyMP client keeps `WorldModel` in `storage['worldModel']` to survive
  this. `[S]`
- Known conflicts: SP's page says it "conflicts with many mods"; the specific
  list was not read. `[U]` Treat gameplay SKSE DLLs as suspects when the
  client crashes on SP startup.
- Linux/Proton: "some crashes can occur on SP startup." `[V]`
- Events/hooks/cosave API: documented in `docs/skyrim_platform/*.md`
  (events.md, hooks.md, browser.md, cosave.md); not read for this skill.
  `[U]` Point the user at those files rather than paraphrasing from memory.

## Troubleshooting SP on a client

1. `SkyrimSE.exe` version is 1.6.1170 (Properties → Details).
2. SKSE 2.2.8 loads (SKSE log in `Documents\My Games\Skyrim Special Edition\SKSE\`).
3. Address Library AE present; Engine Fixes both parts present.
4. SP log in the same folder shows no plugin load errors.
5. `Data/Platform/Plugins/` contains the SkyMP client `.js` and its
   `-settings.txt` with the right server address/port.
6. If Chromium UI is missing, try `BackendName=tilted` explicitly and check
   TCP 3000/8080 reachability from the client.
