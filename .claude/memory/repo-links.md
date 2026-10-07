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

- Repo merge (DECIDED 2026-10-07 by Timothy): no monorepo. Keep three repos (56 open PRs, 266 branches, installed launchers self-update from the launcher repo's releases) and add a launcher<->discord contract test in CI. Plan: /mnt/project-files/aetherial-dawn/cross-repo/monorepo-merge-plan.md

- Contract test (draft discord #5, launcher #56): contract in `contract/launcher-server.json` in discord, identical copy in launcher `contract/`; change both together on the same branch name. Discord CI runs `npm test` incl. `test/contract.test.js`; launcher CI `contract.yml` scans Rust calls against it. `knownGaps` lists login-discord/token, client-status, crash-reports until they reach main: empty it when discord #4 merges. `/faces/*` routes are marked external (who serves them is unconfirmed). Copy-sync check needs repo secret CONTRACT_SYNC_TOKEN (asked of Timothy).

**Why:** a change on one side of these links breaks the other; the first two links come from CONTRACT.md and the READMEs, the third from the hook name and README (not traced in code).

**How to apply:** touching auth, bans or kicks means checking all three repos. Visual map: https://claude.ai/artifact/WbN7rUuZM9qqg8JfMHtePT
