---
name: program-baseline
description: Standing Goal baseline 2026-10-13: where this repo's feature register, 5-tier backlog and evidence checklist live
type: reference
verified: 2026-10-13
refs: docs/program/README.md, docs/program/register.md, docs/program/backlog.md, docs/program/checklist-template.md
---
Timothy's Standing Goal (verbatim in the project folder: /mnt/project-files/aetherial-dawn/standing-goal.md) is active until he retires it. Baseline: docs/program/register.md (63 rows, status ladder Planned -> Implemented -> Offline-tested -> Staging-verified -> Live-verified -> Monitored, plus Blocked), docs/program/backlog.md (findings in his 5 priority tiers, open PRs), docs/program/checklist-template.md (copy into PRs). Cross-repo set: /mnt/project-files/aetherial-dawn/program/.

Suite at baseline: tests not run in cloud (no Rust); PROGRESS reports 217 core tests.
No row is Staging-verified (no staging exists) or Monitored. Installed versions on the VPS and PCs are UNKNOWN unless a row says REPORTED.

**Why:** every cycle must continue from recorded state, not rediscover it; a mock, build or merge is not live verification.
**How to apply:** change a feature -> update its register row and backlog item in the same PR; tick checklist boxes only with evidence; close a backlog item only with a PR, CI run, log or game check named in the item.
