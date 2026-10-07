---
name: live-mod-list
description: The live mod list is fetched at runtime from the server, not stored in any repo; built-in required mods are in modlist.rs
type: reference
verified: 2026-10-07
refs: core/src/modlist.rs, docs/masters-json.md, core/src/serverorder.rs
---
The launcher fetches `<base>/mods.json` at runtime (base `https://vps-d38c928e.vps.ovh.us/launcher`) and caches it on player PCs as `.aetherial-dawn/mods/server-list.json`. Server plugin order is `masters.json` (5 base masters plus lane plugins; see `docs/masters-json.md`). Built-in required client mods are in `core/src/modlist.rs`.

**Why:** the list changes without a launcher release, so a repo search will not find the current mods.

**How to apply:** to see what is live, read the server's mods.json or the player cache, not the repo. Edit `modlist.rs` only for built-in required mods.
