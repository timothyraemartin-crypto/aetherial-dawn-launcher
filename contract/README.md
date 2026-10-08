# Launcher <-> login service contract

This is a copy. The discord repo (aetherial-dawn-discord, `contract/`) owns the file; edit both together.

`launcher-server.json` lists every call the launcher makes to this service: method, path, auth, the
statuses the launcher handles, and the fields it reads (with an example body for each).
CONTRACT.md explains the flows in words; this file is what CI checks.

- **Here:** `test/contract.test.js` starts the real HTTP server and calls each endpoint. A missing route,
  a status the launcher doesn't handle, or a response field that is gone or has the wrong type fails the test.
- **In the launcher repo:** `contract/launcher-server.json` is an identical copy. Its CI scans the launcher's
  source for server calls and fails if one isn't in the file (or the file lists one the launcher no longer makes),
  and a Rust test parses the example bodies with the launcher's own structs.
- **Copies must match.** The `contract-in-sync` job compares this file with the launcher repo's
  (needs the repository secret `CONTRACT_SYNC_TOKEN`; skipped without it). Change both repos together,
  using the same branch name so the job compares the pair.

## Changing the contract
1. Edit `launcher-server.json` here and the same file in the launcher repo.
2. Make the server and the launcher match it; both test suites must pass.
3. Update CONTRACT.md.

## knownGaps
Endpoints the launcher calls that the server doesn't serve yet (empty now that the sign-in endpoints are on main). The test requires them to stay missing, so
when one lands the test fails until you remove it from `knownGaps` in both repos. The list can only shrink;
it should be empty once the sign-in endpoints reach main.

## External endpoints
`"server": "external"` entries (the faces service) are called by the launcher but not served here. They are
only checked on the launcher side. Whoever owns that service should confirm the owner and add a check there.
