# AetherialDawn-CrashLog 1.0.0

Our own small SKSE plugin (no CommonLib, adlite kit, Skyrim 1.6.1170). When Skyrim hits a fatal
exception it writes a short report, so a crash that leaves no CrashLogger file still tells us what died.

Build: `skse-toolchain\build.bat native-mods\crashlog\crashlog.cpp AetherialDawn-CrashLog.dll`
Offline test (no game): `build-test.bat <skse-toolchain> handler_test.exe`, then run
`handler_test.exe handled|quiet|crash|stack <empty dir>` and read `<dir>\AetherialDawn-Crash.log`.

## What it writes
`Documents\My Games\Skyrim Special Edition\SKSE\AetherialDawn-Crash.log` (the run before: `.prev.log`;
a new run starts a new file) and the same lines in `AetherialDawn-CrashLog.log` (the plugin's own log):

1. Exception code and name, address as `module+offset`, read/write address for access violations, and the label
   `UNHANDLED` (the game is going down) or `FIRST-CHANCE` (may be handled; the game may carry on).
2. A `CALL STACK:` of up to 24 frames as `module+offset`, walked from the fault's context with the x64 unwind tables
   (if that gives under 2 frames, `CaptureStackBackTrace` of the report thread is used and the report says so).
3. `ACTIVE MENUS:` read-only from the engine's UI object (same ids/layout NoPause verified); other menus show as
   `other(vtable base+0x...)`; `(unknown: Address Library not available)` when versionlib is missing.
4. `SKSE PLUGINS` (every DLL loaded from `Data\SKSE\Plugins`, base and size) and `OTHER DLLS` (names; Windows system DLLs counted).

## Cases and limits (every case)
| Case | What happens |
|---|---|
| Real crash (nobody handles the exception) | UEF report, `UNHANDLED`, then the previous filter runs; exit code unchanged |
| Exception the game/engine handles itself (AV in a `__try`) | vectored handler writes `FIRST-CHANCE`; max 3 per run, same code+address once |
| C++ throw, debugger/breakpoint exceptions, `OutputDebugString` | ignored (not in the fatal list) |
| Stack overflow | works: the report is built by a worker thread made at load time |
| Crash inside the loader lock / the log file | the faulting thread waits at most 8 s for the worker, then moves on |
| Another plugin kills the process in its own first-in-line vectored handler (e.g. CrashLogger SSE terminates first) | our handler never runs: UNKNOWN until seen in game; CrashLogger's own file still exists then |
| Another plugin replaces the unhandled-exception filter later | UEF report lost; the vectored one still fires for fatal codes |
| `0xC0000409` fast-fail (`__fastfail`) | the process dies without exception dispatch: no report (Windows limitation) |
| Report thread fails | swallowed (`__try`), the game is never affected |

The handler only writes files. It never changes the exception, the stack or the exit code.

## Kill switch
Create an empty file `Data\SKSE\Plugins\AetherialDawn-Crash.off` in the Skyrim folder. Next start the plugin
logs `AetherialDawn-Crash.off found` and installs nothing. The launcher never creates or deletes that file.

## Logs to read
`AetherialDawn-CrashLog.log` first line: `crash handler installed ...` (or why not). `AetherialDawn-Crash.log` only exists after a fatal exception.
The launcher's crash report (staff report and the player's session report) includes `AetherialDawn-Crash.log` in full (12,000 characters at most, paths hidden).

## In-game check (NEEDS_GAME_CHECK, not done)
1. Start Skyrim with the plugin. `AetherialDawn-CrashLog.log` says `crash handler installed`, `open-menu names in the report: on`. No `AetherialDawn-Crash.log` yet.
2. Play 5 minutes with menus (Esc, Inventory, Map, console): still no `AetherialDawn-Crash.log`, or only `FIRST-CHANCE` entries (note which).
3. Create `AetherialDawn-Crash.off`, start the game: log says `found`, no handler. Remove the file again.
4. Provoke a real crash (staff only: alt-F4 is NOT a crash; use a known-bad mod or `coc` into a broken cell): the report has `UNHANDLED`, a stack with `SkyrimSE.exe+0x...`, the SKSE plugin list, and the launcher's crash report contains it.
5. Check Skyrim's frame rate / load times are the same with and without the plugin.
