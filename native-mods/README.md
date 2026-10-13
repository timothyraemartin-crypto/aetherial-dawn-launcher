# Native mods (our own SKSE plugins)

Source for the DLLs the launcher bundles under `src-tauri/bundled/Data/SKSE/Plugins/`.
Compiled DLLs live only in `bundled/`; they are not duplicated here.

- `nopause/` AetherialDawn-NoPause (1.0.8): keeps the game world running while menus are open (see docs/NOPAUSE-PLAN.md).
- `quiet-pickups/` AetherialQuietPickups: silences pickup notifications.
- `crashlog/` AetherialDawn-CrashLog (1.0.0): writes `AetherialDawn-Crash.log` on a fatal exception; the launcher adds it to the crash report.

