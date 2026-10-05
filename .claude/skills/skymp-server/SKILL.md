---
name: skymp-server
description: Expert knowledge for building, configuring, scripting and troubleshooting SkyMP (Skyrim Multiplayer, github.com/skyrim-multiplayer/skymp) servers and clients, with a focus on Linux hosting and mod compatibility. Use this skill whenever the user mentions SkyMP, Sky MP, skymp5-server, skymp5-client, skymp5-gamemode, SkyrimPlatform (SP), server-settings.json, loadOrder, the mp scripting API, server-side Papyrus, a Skyrim multiplayer or roleplay server, mods breaking or crashing a Skyrim server, ESL plugins on a server, or Skyrim version pinning/downgrading for multiplayer. Trigger even when the user just says "my server" or "the mod broke it" in a Skyrim context. Not for Skyrim Together, which is a different project.
---

# SkyMP Server Skill

SkyMP is a client-server multiplayer mod for Skyrim Special Edition. The server
(`skymp5-server`) is a Node.js/TypeScript host around a C++ core that parses the
game's plugin files itself, owns the authoritative world state, runs its own
Papyrus VM, and persists changes to a database. The client (`skymp5-client`) is
TypeScript running inside the real game on SkyrimPlatform (SP), an SKSE plugin
that embeds a JS runtime and Chromium.

Most "a mod broke the server" problems come from three structural facts the
project rarely spells out:

1. **The server parses every plugin in `loadOrder` itself** (via libespm). A
   plugin the parser can't handle kills the server at startup. The last
   `[ESPM] <plugin> read in ...` line before the crash names the culprit.
2. **ESL / light plugins are not supported server-side.** Upstream issue #530
   ("ESL plugins cannot be used") was labelled "temporarily unsupported" and
   closed without a fix. This includes ESL-flagged `.esp` files and all
   Creation Club / Anniversary Edition content.
3. **Server and clients must have identical plugins, identical order, identical
   bytes.** Load-order position decides FormIDs. The server writes
   `data/manifest.json` with crc32/size per plugin; any drift means clients and
   server point at different objects.

A fourth fact explains the "it loads but doesn't work" cases: **Skyrim's own
simulation does not run on the server.** Server-side Papyrus is a custom VM with
a subset of natives. Quest logic, AI packages, SKSE-dependent scripts and most
script-driven mod features either run only locally (and desync) or not at all.

## Verification markers

Reference files tag claims so you know how much to trust them:

- `[V]` verified from a primary source (repo file, official docs, vendor page)
- `[S]` secondary (DeepWiki, forks, community posts)
- `[I]` inferred from verified mechanisms
- `[U]` unverified — checked, but no primary source could be read

Never present an `[U]` item as fact to the user. Say "the docs suggest" or
"verify in `skymp5-server/ts/settings.ts`". If the user has the repo checked
out, offer to read the file and settle it. Never invent `server-settings.json`
keys, `mp.*` methods, or error strings that are not in the references.

## Always do

- Pin every client to **Skyrim SE/AE 1.6.1170** with **SkyrimPlatform 2.9.0**
  and **SKSE64 2.2.8**. Bethesda's 1.7.99 / 1.7.104 (August 2026) are not
  supported by SP. See `references/versions.md`.
- Copy the server's `Skyrim.esm`, `Update.esm` and DLC masters from a
  **1.6.1170** install, never from a 1.7.x install.
- Keep `loadOrder` identical to the clients' `plugins.txt`, case-exact
  (Linux filesystems are case-sensitive), vanilla masters first.
- Set `databaseDriver` explicitly. The two official docs disagree on the
  default (`file` vs `sqlite`).
- Use strict JSON in `server-settings.json`. Comments break it.
- Start every mod test from a vanilla baseline (five masters, minimal
  gamemode) and add plugins back in halves.
- Back up `world/` (or `mongodump`), `server-settings.json` and the gamemode
  before every update. Unpacking a new `dist` overwrites them.
- Turn `offlineMode` off and `isPapyrusHotReloadEnabled` off before going
  public.
- Read the repo's `CLAUDE.md` when editing SkyMP source itself.

## Never do

- Never put `.esl` files, ESL-flagged `.esp` files, or Creation Club content
  in `loadOrder`.
- Never tell the user Skyrim 1.7.x works with SkyMP.
- Never set `ip` to `"0.0.0.0"`. Omit `ip` to use the public address.
- Never build the server with GCC, or on Alpine / Arch. Use Ubuntu with clang
  (the official Dockerfile uses Ubuntu 25.10 + clang-20 + Node 22).
- Never pull the `skymp/skymp-server` Docker image. It is years stale.
- Never assume a script-driven mod (quests, followers, survival, overhauls)
  will sync just because it loads.
- Never edit `data/manifest.json` or `data/_libkey.js`.
- Never change `loadOrder` on a live world without a backup. FormIDs shift.

## Triage workflow

When the user reports a problem, classify it first, then open the matching
reference. Ask for the server log tail if they haven't pasted it.

| Symptom | Go to |
|---|---|
| Server exits/hangs during startup, last log line is `[ESPM] ...` | `references/mod-compatibility.md` → "Diagnosis workflow" |
| A mod loads but items/NPCs/quests don't work or desync | `references/mod-compatibility.md` → compatibility table |
| Players can't connect, blank UI, port errors | `references/troubleshooting.md` |
| Client crash on launch, SKSE/SP version errors | `references/versions.md` then `references/skyrimplatform.md` |
| Build fails on Linux | `references/linux-setup.md` |
| Writing or debugging gamemode / `mp.*` code | `references/scripting-api.md` |
| Papyrus "Method not found" / stack errors on server | `references/scripting-api.md` → Server-side Papyrus |
| Which key does what in server-settings.json | `references/server-settings.md` |
| Database, backups, migration | `references/linux-setup.md` → Persistence |
| Anything not covered, or an `[U]` item | `references/sources.md` → verification map |

### Mod-breakage fast path

1. Confirm versions: every client on 1.6.1170, SP 2.9.0 loads cleanly
   (SP log: `Documents\My Games\Skyrim Special Edition\SKSE\`).
2. Run `scripts/scan_plugins.py <dataDir>` to flag `.esl` files, ESL-flagged
   ESPs, header version 1.71, and plugins whose masters are missing or out of
   order.
3. Run `scripts/check_loadorder.py` against the client's `plugins.txt` and the
   server's `server-settings.json` (and `manifest.json` if present) to catch
   order and checksum drift.
4. If both scripts pass and the server still dies, binary-search the plugin
   list from a vanilla baseline. Reproduce with vanilla + one plugin.
5. If the server runs but behaviour is wrong, ask what the mod *does*. If the
   answer involves scripts, quests, AI, SKSE or MCM logic, it is a sync
   problem, not a load problem. Suggest reimplementing the needed piece
   server-side in the gamemode (see `references/scripting-api.md`).

## Reference files

- `references/versions.md` — Skyrim/SP/SKSE version matrix, downgrade
  procedure, why 1.7.x is out.
- `references/architecture.md` — repo layout, components, sync model,
  networking, persistence, FormID rules.
- `references/linux-setup.md` — Docker and from-source build, dist layout,
  start command, systemd unit, firewall, data directory, database drivers
  and backups, player-side install checklist.
- `references/server-settings.md` — complete key table with verification
  status, annotated example, CLI overrides, ports.
- `references/mod-compatibility.md` — the hard rules, a compatibility table
  by mod category, the diagnosis checklist, a baseline modlist, how to
  distribute a modpack with parity checks.
- `references/troubleshooting.md` — symptom → cause → fix table with exact
  strings where known.
- `references/scripting-api.md` — the `mp` API, `ctx` fields, event
  sources, patterns and anti-patterns, server-side Papyrus natives.
- `references/skyrimplatform.md` — SP versions, install layout, ini options,
  release-note highlights.
- `references/sources.md` — every source URL, the unverified list, and the
  exact repo files to read to settle each open question. Also how to update
  this skill when a new fix is learned.

## Scripts

- `scripts/scan_plugins.py` — reads TES4 headers of every `.esp/.esm/.esl`
  in a folder and reports ESL flags, header version, masters, and ordering
  problems against a `loadOrder`. Pure Python, no dependencies.
- `scripts/check_loadorder.py` — compares a client `plugins.txt` with the
  server's `loadOrder`, and optionally verifies file crc32/size against
  `manifest.json`. Pure Python.

Run them with `python3 scripts/<name>.py --help`. Prefer running a script over
reasoning about a plugin list by hand; the ESL flag in particular is invisible
from the filename.

## Assets

- `assets/server-settings.example.json` — strict-JSON template with only
  verified keys plus clearly marked unverified ones.
- `assets/skymp.service` — systemd unit template.

## Working with the user's code

- When the user pastes a gamemode or client snippet, check it against the
  patterns in `references/scripting-api.md` before suggesting changes.
  The common defects are: trusting client-sent values, heavy work in
  `on("update")` snippets, unnamespaced `ctx.state` keys, and calling
  Papyrus natives the server VM doesn't implement.
- When the user pastes a `server-settings.json`, validate it as strict JSON,
  check every key against `references/server-settings.md`, and flag any key
  that is `[U]` so they can confirm it in `settings.ts`.
- When the user describes a modlist, run through the compatibility table
  category by category rather than guessing per mod. Named mod-level
  reports are thin; category-level reasoning from verified mechanisms is
  what this skill is good at.

## Keeping this skill current

When a problem is solved in conversation that this skill did not predict, or
an `[U]` item gets settled, append the finding to the matching reference file
with today's date and a `[V]`/`[S]` marker, and update
`references/sources.md`. Keep SKILL.md itself short; details belong in
references. See the "Updating this skill" section of `references/sources.md`.
