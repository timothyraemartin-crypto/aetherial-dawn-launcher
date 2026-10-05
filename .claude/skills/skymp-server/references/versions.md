# Versions, pinning and downgrading

Status as of 2026-10-05. Re-check the SkyrimPlatform Nexus page (mod 54909)
and `skyrim-platform/README.md` before quoting support claims; a new SP
release could change the picture.

## The one-line rule

Target **Skyrim SE/AE 1.6.1170** on every client, with **SkyrimPlatform
2.9.0** and **SKSE64 2.2.8**. The server does not run `SkyrimSE.exe` and does
not care about the runtime version, but its master files must come from the
same 1.6.1170 install the clients use.

## Skyrim runtime timeline (Steam)

| Runtime | Released | SKSE64 | SkyMP/SP status |
|---|---|---|---|
| 1.7.104 | 2026-08-27 | 2.3.1 | Not listed by SP. Unsupported. `[V]` |
| 1.7.99 | 2026-08-20 | 2.3.0 | Not listed by SP. Unsupported. `[V]` |
| 1.6.1170 | 2024-01-17 | 2.2.6 / **2.2.8** | **Supported.** SP README: "Steam latest (currently Skyrim SE/AE 1.6.1170)". SP 2.9.0 notes: "Added support for Skyrim 1.6.1170.0". `[V]` |
| 1.6.1130 | 2023-12-05 | — | Not listed by SP. Superseded within weeks. `[V]` |
| 1.6.640 | 2022 | 2.2.3 | Supported (added in SP 2.7.0). `[V]` |
| 1.5.97 | 2019 | 2.0.20 | "Deprecated, not accepting bugs anymore" per SP README. `[V]` |

The 1.7.x updates also broke SKSE DLL plugins built on CommonLibSSE-NG; the
Address Library author warned that plugins need to update CommonLib and
recompile. So a 1.7.x modlist is unstable in general as of October 2026, not
just for SkyMP. `[V]`

## SKSE64 for 1.6.1170

The SKSE64 Nexus files page (mod 30379) marks two builds as compatible with
1.6.1170: **v2.2.6** (2024-01-17) and **v2.2.8** (2026-08-20, listed under
"Old files" after 2.3.x shipped). Use **2.2.8**, the newer build for that
runtime. `[V]`

## SkyrimPlatform and the SkyMP client

- SP 2.9.0 is the current release (Nexus 54909). `[V]`
- The SkyMP client requires SP 2.9.0 (`skymp5-client/src/version.ts`). `[S]`
- SP 2.9.0 release notes say "New game required." Start a new save/profile
  after installing or updating SP. `[V]`
- Also install Address Library (AE all-in-one) and SSE Engine Fixes. SP's
  README says without Engine Fixes the "Chromium process will hang". `[V]`

## Downgrade procedure (1.7.104 → 1.6.1170)

1. In Steam, set Skyrim SE to "Only update this game when I launch it", and
   launch via SKSE only. Back up the game folder first.
2. Pick one of:
   - **Best of Both Worlds Downgrade Patcher** (Nexus SSE mod 169962). File:
     "1.7.104 to 1.6.1170 Downgrade Patcher" (v1.7.104.3, 2026-08-30). If
     still on 1.7.99, use the 1.7.99 → 1.6.1170 file. Put the exe next to
     `SkyrimSE.exe` and run it. `[V]`
   - **SDT – Skyrim Downgrade Tool** (Nexus 188916), a .BAT that downgrades
     "from ANY version to 1.6.1170" and was confirmed working after the
     Aug 27 hotfix. `[V]`
   - Manual Steam depot download via the Steam console (`steam://nav/console`),
     depots land in `Steam/steamapps/content/app_489830`. Assemble into a
     separate stock-game folder. Most work, most control. `[V]`
3. Verify: right-click `SkyrimSE.exe` → Properties → Details → version
   1.6.1170.0. Do this on **every** player's machine before debugging
   anything else.
4. Install in order: SKSE64 2.2.8 → Address Library (AE) → SSE Engine Fixes
   (both parts) → SkyrimPlatform 2.9.0 → SkyMP client → the server's plugin
   pack.
5. If anyone uses Creation Kit, "Creation Kit Downgrade Patcher (1.7.99)"
   (Nexus 190110) rolls CK back to 1.6.1378.1 to match 1.6.1170. `[V]`

## Why the server's masters must come from 1.6.1170

The 1.7.x updates changed master files. A post on USSEP's Nexus page warns
that "Bethesda made some substantial navmesh edits to Blackreach in this
update. A navmesh edge link mismatch will crash the game when you enter that
cell", and a September 2026 sticky describes a full revamp of Blackreach
navmeshes. `[V]` If the server parses 1.7.x masters while clients run 1.6.1170
masters (or vice versa), the crc32 values in `data/manifest.json` will differ
and reference IDs can diverge. Whether `Skyrim.esm`/`Update.esm` byte-changed
is `[U]`; the safe rule is to copy from the same install and let the manifest
checksums catch drift.

## USSEP, Creation Club, Anniversary Edition

- USSEP itself is a full `.esp`, so the server can parse it. But on AE
  installs USSEP depends on the free Creation Club `.esl` masters, which the
  server cannot load. Check its masters in xEdit before adding it. `[I]`
- All CC/AE content ships as `.esl`. Assume it is unusable server-side until
  ESL support lands upstream. `[V]` (issue #530) + `[I]`
- "Backported ESL support" mods (Nukem9) patch the *game* to read 1.71-header
  ESLs on older runtimes; they do nothing for the SkyMP server's parser. `[I]`

## What to re-check periodically

- Nexus 54909 (SkyrimPlatform) files tab: any release listing 1.7.x support.
- `skyrim-platform/README.md` "Supported versions" section.
- SKSE64 Nexus 30379 files tab for the newest 1.6.1170-compatible build.
