# Native mods (our own SKSE plugins)

Source for the DLLs the launcher bundles under `src-tauri/bundled/Data/SKSE/Plugins/`.
Compiled DLLs live only in `bundled/`; they are not duplicated here.

- `nopause/` AetherialDawn-NoPause (1.0.8): keeps the game world running while menus are open (see docs/NOPAUSE-PLAN.md).
- `quiet-pickups/` AetherialQuietPickups: silences pickup notifications.
- `skill-tree/` AetherialSkillTree (source 0.1.4; bundled DLL still 0.1.2 until built): skill tree menu data and hooks.

Build: `skill-tree/build.bat <source.cpp> <output.dll>` (MSVC 2022, plus the local `skse-toolchain` MinHook copy, which is not stored in this repo).
