---
name: signed-feeds
description: Server mods.json and client manifest need an Ed25519 .sig; re-sign after every edit (draft PR #58)
type: howto
verified: 2026-10-07
refs: docs/masters-json.md
---
Draft PR #58 (CI green, mergeable, waiting on Timothy; not merged): `mods.json` and `client/manifest.json` need an Ed25519 signature (`<file>.sig`) from the pinned key, the same server key already pinned for gamemode scripts. No private key is in any repo. The server address must be https. Direct (non-Nexus) downloads must pin a sha256 or they are dropped; Nexus entries stay optional. The manifest `remove` list only deletes files the launcher synced. Code: `core/src/feedsig.rs`, `core/src/bin/sign-feed.rs`; doc `docs/signing-feeds.md` (after #58 merges).

Rollout: a launcher that has never seen a valid signature accepts unsigned files; once it has seen one, it requires signatures for that file. `REQUIRE_SIGNED` is a later switch.

**Why:** the review found mod list and manifest unsigned, with hashes from the same host as the files.

**How to apply:** after ANY edit on the VPS, run `sign-feed sign <key.pem> <web root>` and `sign-feed verify`, or signed launchers will refuse the file. Before release, add a sha256 to every `mods.json` url entry without `nexus`. Open: the src-tauri edits are not built in CI until #59 lands; rebase #58 on #59 then. Not covered: replay of old signed files, `masters.json`, `server-lane.json`, patch files.
