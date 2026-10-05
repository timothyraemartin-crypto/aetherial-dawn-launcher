# Mod compatibility

## Contents
1. The six hard rules
2. Compatibility table by mod category
3. Named reports and how solid they are
4. Diagnosis workflow (checklist)
5. Recommended baseline modlist
6. Distributing a modpack with parity checks
7. Converting an ESL mod to a full plugin

## 1. The six hard rules

1. **Same plugins, same order, same bytes.** README: "Use esp/esm mods, just
   ensure both client and server load order are the same." The server
   writes `data/manifest.json` with crc32 and size per plugin. An xEdit
   clean done on one machine only, or a different mod version on one client,
   breaks parity. `[V]`
2. **Every plugin in `loadOrder` must exist in `dataDir`, with its masters
   listed earlier.** Case-exact on Linux. `[V]`
3. **No ESL / light plugins on the server.** Issue #530 "ESL plugins cannot
   be used" (opened 2021-11-12) was labelled "temporarily unsupported" and
   closed in bulk ("Closing all 'temporary unsupported'") with no fix. This
   covers `.esl` files, ESL-flagged `.esp` (ESPFE), FE-prefixed FormIDs and
   Creation Club content. Since game 1.6.1130, header version 1.71 light
   plugins may use FormIDs xx000–xxFFF, which older parsers don't expect. `[V]`
4. **Masters from 1.6.1170 only.** See `versions.md`. `[V]`+`[I]`
5. **Assets ship to clients, not the server.** "currently .bsa archives are
   used only on the client-side", named after their plugin. `[V]`
6. **Scripts that should be authoritative go in `data/scripts` on the
   server and must only use natives the server VM implements.** Client-side
   Papyrus runs locally and is not synced. `[V]`+`[I]`

## 2. Compatibility table by category

Verdicts: **Works** / **Works with caveats** / **Breaks gameplay (desync)** /
**Breaks server** / **Breaks client** / **Unknown**. Reasoning is from the
verified mechanisms above; mod-by-mod field reports are thin (see §3).

| Category | Verdict | Why |
|---|---|---|
| Texture/mesh/sound replacers, ENB/ReShade (no plugin) | Works, client-only | Nothing for the server to parse. Mixed visuals across players are fine. |
| SKSE64, Address Library, SSE Engine Fixes | Required on client | SP is an SKSE plugin; SP's README recommends Engine Fixes. |
| New items/weapons/armor as full `.esp/.esm` | Works with caveats | Must be on server + every client, same order. Items sync via inventory. |
| Leveled list / crafting / balance record edits | Works with caveats | Data only. Only effects the server implements will show (e.g. crafting menus are client-side; results are validated by server inventory sync). |
| Cell/worldspace edits, new lands, navmesh, persistent refs | Works with caveats / risky | New references become server objects. Big plugins slow startup; a malformed record can crash libespm. Test alone from vanilla. |
| Overhauls (Requiem, Ordinator, perk/combat rewrites) | Breaks gameplay / Unknown | Perk entry points and scripts run client-side or not at all. Desync likely. Server stability unverified. |
| Quest/dialogue mods, follower mods, follower frameworks | Breaks gameplay (desync) | Quests, AI packages and dialogue scripts are not server-simulated. |
| Survival/needs mods (script-driven) | Breaks gameplay (desync) | Each client runs its own copy; no shared state. |
| Papyrus-heavy mods with scripts placed server-side | Breaks server (risk) | Limited natives. A vanilla `ResourceFurnitureScript` produced "Method not found - 'RegisterForAnimationEvent'" and "[onActivate] Error: Out of stack space" then a server crash (old issue mirror). A regression test `misc/tests/test_dlc1chauruscocoonscript_stack_overflow.js` exists for a vanilla DLC1 script overflow. `[S]` |
| SKSE DLL plugins (gameplay) | Client-only, Unknown | Never loaded by the server. Can conflict with SP ("SkyrimPlatform conflicts with many mods"). Must be built for 1.6.1170. |
| Animation frameworks (Nemesis, FNIS, Pandora, OAR/DAR), XPMSSE | Works with caveats, client | No server records, but remote players' animations arrive as animation events. Every player needs the same behavior files or others T-pose / show wrong animations. `[I]` |
| Physics (HDT-SMP, CBPC), body replacers | Works, client | Local rendering. Keep body meshes consistent for appearance sync. |
| UI mods (SkyUI, MCM) | Works with caveats | SP notes "Limited MCM support". SkyUI's `.esp` must be in the server `loadOrder` if clients have it. TESAll reports SkyUI preinstalled on some servers. `[S]` |
| Creation Club / AE content (`.esl`) | Breaks server | Rule 3. |
| ESL-flagged / compacted ESPs | Breaks server | Rule 3. |
| Mods requiring 1.7.x or new CommonLib builds | Breaks client | SP has no 1.7.x support. |
| USSEP | Works with caveats | Full `.esp`, but on AE it depends on CC `.esl` masters. Check masters in xEdit. `[I]` |

## 3. Named reports and how solid they are

- **ESL plugins**: officially unsupported (issue #530). Solid. `[V]`
- **Vanilla chopping-block script crash** (`0x6c3c9`, Riverwood), seen on a
  githubhelp mirror of an old issue. Current status unknown. `[S]`
- **Keizaal Online** (skyrim-roleplay/skymp fork) reached ~650 concurrent
  players in April 2026 and removed all base-game human NPCs. Shows the
  server scales when the world is simplified. `[S]` (Dexerto, TheGamer)
- **TESAll (RU)**: mods adding weapons, armor and locations work; mods are
  "partially supported" because "some functions may not work, e.g.
  Papyrus". `[S]`
- **Skyrim Together Reborn's blocklist** (Skyrim Souls RE, DLL Plugin
  Loader, at one point Engine Fixes) is a **different project** and does not
  transfer. SP recommends Engine Fixes. `[V]`
- No named third-party `.esp` or DLL was confirmed in primary sources as
  SkyMP-breaking. Reason from categories, and record new findings in this
  file with a date.

## 4. Diagnosis workflow (checklist)

1. **Version gate.** Every client on 1.6.1170; SP 2.9.0 loads; SP log in
   `Documents\My Games\Skyrim Special Edition\SKSE\` has no errors.
2. **Scan the plugins.** `python3 scripts/scan_plugins.py <dataDir>
   --settings server-settings.json`. Fix anything it flags (ESL flag, 1.71
   header, missing/out-of-order masters, files absent from `dataDir`).
3. **Check parity.** `python3 scripts/check_loadorder.py --plugins
   <client plugins.txt> --settings server-settings.json --manifest
   data/manifest.json --client-data <client Data folder>`.
4. **Vanilla baseline.** Five masters only, empty or minimal gamemode,
   `isPapyrusHotReloadEnabled: false`. Start the server, connect one client.
5. **Read the `[ESPM]` lines.** On a crash, the last plugin logged is the
   suspect.
6. **For each suspect, in xEdit/SSEEdit:** check header flags (ESL), header
   version (1.71), compacted FormIDs, masters present and earlier, "Check
   for Errors" for unresolved references and deleted navmeshes.
7. **Binary search.** Add half the remaining plugins, restart, repeat until
   one plugin is isolated. Reproduce with vanilla + that one.
8. **Script check.** If the crash follows an activation or event, look for
   "Method not found" or stack errors in the server log. Remove that mod's
   `.pex` from `data/scripts` or reimplement its logic in the gamemode.
9. **Runtime desync.** Server fine, players see different things → the mod
   depends on simulation (quests, AI, scripts). Drop it or move its state
   into server properties (`scripting-api.md`).
10. **Record the result** in §3 of this file with the date and a marker.

## 5. Recommended baseline modlist

- **Client:** SKSE64 2.2.8 → Address Library (AE) → SSE Engine Fixes (both
  parts) → SkyrimPlatform 2.9.0 → SkyMP client → server plugin pack →
  optional client-only visuals (textures, ENB, physics).
- **Server:** the five vanilla masters, then the same full (non-ESL) plugins
  in the same order.
- Start with zero gameplay plugins. Add content packs (items, locations)
  one at a time with a restart each. Add script-driven features only as
  gamemode code.

## 6. Distributing a modpack with parity checks

- Ship one versioned archive or MO2 profile containing plugins + BSAs. No
  FOMOD options that change plugin files.
- Publish `SHA256SUMS` next to it; bump a version string in the server name
  or MOTD on every change.
- A "stock game" folder pinned to 1.6.1170 (Wabbajack or MO2) prevents Steam
  from updating players out from under you.
- Fork pattern worth copying: skymp-heavy-rp exposes a `/mods.json` parity
  endpoint (port 7758) and its launcher refuses to connect on mismatch;
  Nirn RP's launcher installs a "signed modpack" and checks every file.
  `[V]`/`[S]` The server's own `manifest.json` crc32 values are the
  reference your launcher or script should compare against.

## 7. Converting an ESL mod to a full plugin

Only if the mod is data-only and worth a full slot (254 max):
1. Open in xEdit. If the plugin's FormIDs were compacted into the light
   range, it is still safe to un-flag as long as you keep the same FormIDs
   on both sides; do **not** renumber.
2. Clear the ESL flag in the TES4 header (or re-save as `.esp` without the
   flag). Keep the filename the same, `.esp` extension.
3. Put the identical file on the server and on every client, same position.
4. Re-run `scripts/scan_plugins.py` to confirm no light flag remains.
5. Expect a new `manifest.json`; redistribute the pack.
