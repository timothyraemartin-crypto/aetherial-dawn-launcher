# 2026-10-13 CI hardening

CI hardening (background audit): PR-triggered jobs on the self-hosted runner (linux-checks, contract, scripts) skip fork PRs; bug-investigate gets kill switch BUG_INVESTIGATE=off. Workflow-only; not yet run on a fork PR.
