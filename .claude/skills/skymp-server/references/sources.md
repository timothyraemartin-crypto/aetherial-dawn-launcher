# Sources, open questions, and how to update this skill

Research dates: 2026-10-04 and 2026-10-05. Repo commit referenced by
DeepWiki: `47ea52f5`.

## Primary sources (repo: github.com/skyrim-multiplayer/skymp, main)

- `README.md` — "Use esp/esm mods, just ensure both client and server load
  order are the same"; "Mostly server-controlled game state".
- `CONTRIBUTING.md` — Linux build (Docker image, clang, no GCC/Alpine/Arch),
  resource needs, test rules.
- `Dockerfile` — Ubuntu 25.10, Node 22, clang-20, vcpkg cache stages.
- `docs/docs_server_configuration_reference.md` — verified settings keys,
  "sqlite by default" claim.
- `docs/docs_database_drivers.md` — file/zip/mongodb/migration, "file driver
  by default".
- `docs/docs_server_data_directory.md` — data layout, BSA rule, manifest.
- `docs/docs_server_ports_usage.md` — port table.
- `docs/docs_server_command_line_api.md` — the four CLI flags.
- `docs/docs_serverside_scripting_reference.md`,
  `docs/docs_events_system.md` — `mp` API, `ctx`, events.
- `docs/skyrim_platform/ini_settings.md` — SP ini.
- `docs/release/sp-2.6.md`, `sp-2.7.md`, `sp-2.9.md` — SP release notes.
- `skyrim-platform/README.md` — supported runtimes, Engine Fixes note,
  save warning.
- `ROADMAP.md` — sync status.
- Issue #530 "ESL plugins cannot be used."
  https://github.com/skyrim-multiplayer/skymp/issues/530
- Issue #111 "Tests: Unknown bind__() error 98"
  https://github.com/skyrim-multiplayer/skymp/issues/111

Mirrors and secondary:
- GitBook mirror of docs: https://pospelovlm.gitbook.io/skyrim-multiplayer-docs
- DeepWiki (AI-generated from the repo): https://deepwiki.com/skyrim-multiplayer/skymp
- Docker Hub namespace: https://hub.docker.com/u/skymp
- skymp5-functions-lib: https://github.com/GMati13/skymp5-functions-lib
- skymp5-scripts: https://github.com/alekcey0211/skymp5-scripts
- Forks with operator notes: vinicius3232/skymp-heavy-rp (parity endpoint,
  AUTH-01 note), ELFREAL/secret-skyrim-mp (offlineMode warning, RU),
  skyrim-roleplay/skymp (Keizaal Online), NatPiercii/alduinak (dist layout),
  EvilPatrick06/skymp (Thornswood), Red House Nexus 52149 (start commands).
- Nexus: SkyrimPlatform 54909, SKSE64 30379, Address Library 32444, SSE
  Engine Fixes 17230, Best of Both Worlds downgrader 169962, SDT 188916,
  CK downgrader 190110, USSEP 266 (Blackreach navmesh note).
- Community: TESAll.club (RU, mods "partially supported"), Nirn RP (signed
  modpack launcher), Dexerto/TheGamer (Keizaal ~650 players, April 2026),
  dyndolod.info (ESL/0xFE background), Nukem9 backported-ESL README.
- Skyrim Together Reborn compatibility lists: **different project**, do not
  transfer.

## Open questions `[U]` and exactly where to settle them

| Question | File(s) to read |
|---|---|
| Complete settings key list, remote GitHub settings merge, env vars | `skymp5-server/ts/settings.ts` |
| Default database driver; file driver on-disk format; Mongo collections | `skymp5-server/ts/settings.ts`, `skymp5-server/cpp/server_guest_lib/database_drivers/DatabaseFactory.cpp`, `FileDatabase.cpp`, `MongoDatabase.cpp` |
| How libespm handles the ESL flag (0x200), 0xFE indices, header 1.71 | `libespm/` (grep `0x200`, `0xFE`, `esl`, `light`), issues after #530 |
| Does the client resolve FE-prefixed IDs | `skymp5-client/src/` (grep `0xfe`, `light`) |
| Full `mp` method/property/event list; desc string format | `skymp5-server/cpp/addon/ScampServer.cpp`, TS typings under `skymp5-server/ts/` |
| Papyrus VM error strings; Linux compile path | `papyrus-vm/` (grep log strings), `build/dist/papyrus/`, `cmake/` |
| Message types; client load-order mismatch behaviour and string | `skymp5-server/cpp/mp_common/` (`MsgType.h`), `skymp5-client/src/services/` |
| Transport library, tick rate, packet limits | `skymp5-server/cpp/mp_common/Networking.cpp`, `vcpkg.json` |
| `.nvmrc`, CI versions, `package.json` start script | `.nvmrc`, `.github/workflows/*.yml`, `skymp5-server/package.json` |
| SP events/hooks/cosave; SP known-conflict list | `docs/skyrim_platform/*.md`, Nexus 54909 description/posts |
| Did `Skyrim.esm`/`Update.esm` change in 1.7.x | compare crc32 from `manifest.json` across installs |
| 2023–2026 issue sweep (crashes, Mongo, desync, Linux) | GitHub issues filtered by label/date |
| Real-server RAM/CPU per player | operator reports (VK/Discord), none found |

When the user has a checkout, prefer reading the file over quoting this
skill. Then update the relevant reference and move the item out of this
table.

## Updating this skill

The user wants the skill to learn from real incidents. When a problem gets
solved in conversation:

1. Identify the reference file that *should* have predicted it.
2. Append a dated line under the closest heading, e.g.
   `- 2026-11-02 [V] Plugin X crashed the server; cause: ESL flag set via
   Wrye Bash. Fixed by un-flagging in xEdit on both sides.`
3. If the fix contradicts an existing line, replace the line rather than
   adding a second opinion; keep the old text in a `(previously: …)` note.
4. If an `[U]` item got settled, change its marker and remove it from the
   table above.
5. Keep `SKILL.md` under ~300 lines; detail belongs in references.
6. Re-zip the `skymp-server/` folder (folder at the zip root, name
   unchanged) and re-upload. On claude.ai: Customize > Skills > "+" >
   Create skill > Upload a skill. Re-uploading under the same name replaces
   the previous version; delete the old one if both show.

Suggested wording for the user to trigger an update: "Update the SkyMP skill
with what we just learned about <X>." Claude then edits the reference text
and returns a new `.skill`/zip.
