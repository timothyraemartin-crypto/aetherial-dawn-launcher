---
name: repo-links
description: How the three aetherial-dawn repos connect (launcher, discord gate, ingame-ui)
type: reference
verified: 2026-10-07
refs: docs/masters-json.md
---
- launcher -> discord: Discord sign-in and `/api/users/me`, per `aetherial-dawn-discord/CONTRACT.md`.
- ingame-ui -> discord: a ban or kick calls the Discord gate (`banUrl`, see ingame-ui `gamemode/ad-ui.js`).
- discord -> ingame-ui: `gamemode-kick-hook.js` (in discord) is loaded by the game server gamemode.
- launcher <-> ingame-ui: no direct link confirmed.

- Repo merge (undecided as of 2026-10-07): the plan recommends NOT merging yet (56 open PRs, 266 branches, installed launchers self-update from the launcher repo's releases) and instead adding a launcher/discord contract test in CI. Plan: /mnt/project-files/aetherial-dawn/cross-repo/monorepo-merge-plan.md

**Why:** a change on one side of these links breaks the other; the first two links come from CONTRACT.md and the READMEs, the third from the hook name and README (not traced in code).

**How to apply:** touching auth, bans or kicks means checking all three repos. Visual map: https://claude.ai/artifact/WbN7rUuZM9qqg8JfMHtePT
