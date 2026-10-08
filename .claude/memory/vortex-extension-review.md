---
name: vortex-extension-review
description: Quality review of vortex-extension/ (verdict B+) and its open robustness findings
type: reference
verified: 2026-10-07
refs: vortex-extension/index.js, vortex-extension/jobs.js, vortex-extension/test/jobs.test.js
---
Verdict (2026-10-07, read-only review, code unchanged): good for pre-alpha. HMAC-SHA256 signing is constant-time and checked before parsing, replay protection is sound (+-60s, single-use nonces), `status` is the only verb and nothing writes to Vortex. 8/8 extension tests pass. Not tested against a real Vortex or Windows; attribute names marked `(verify)` in jobs.js are unproven.

Open findings, in suggested fix order:
1. A signed request with body `null` throws in `jobs.js` and the HTTP request hangs (unhandled rejection in Vortex). Reject non-objects; wrap the `end` handler in try/catch.
2. `index.js` server start and port-file write have no try/catch or `server.on('error')`; a Windows rename conflict could throw inside Vortex.
3. Token is read once at startup, so after a re-pair every request is `unsigned` until Vortex restarts.
4. `status` replies are unsigned and a stale `port` file is never deleted (low risk, same-user only).
5. No test covers `index.js` (404s, oversize body, port file).

**Why:** none breaks the signing or read-only guarantees; all are robustness gaps.

**How to apply:** fix 1-3 together with an `index.js` loopback test. Full report: /mnt/project-files/reports/vortex-extension-quality.md (project files, not in the repo).
