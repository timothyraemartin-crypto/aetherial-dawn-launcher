# Vortex collection as the client install path: revised Package C design

Status: **draft for review** (ingame-ui PR #7, re-scope 5876088533 and addendum 5876094699).
Nothing here is released, published or live. Timothy decides:
- collection publication (including Unlisted);
- launcher merge and release;
- migration of his own profile;
- the server cutover.

Anything marked **(verify)** is read from Vortex's source or documentation, and has not yet been proven on installed Vortex 2.7.1. The disposable-profile evidence gate proves it before anything relies on it.

## 0. Milestones, in order (Timothy, PR #7 5876324959)

1. **Timothy's existing "Aetherial Dawn" profile, installed and validated.**
   The Mods chat owns this milestone:
   - inventory, backup and a proven restore;
   - curating the exact client set;
   - installing it in reviewed batches through Vortex into the existing profile;
   - retiring duplicate launcher-direct files.

   The launcher's part is the read-only status and Play gate. It must say how far the profile is from the client set, from Vortex's own state. It never counts the launcher's direct-to-Data ledger as readiness. Today's 11-row profile reads as incomplete.
2. **Our own collection, built From Profile** after a frozen source-profile receipt. It stays local and editable, and is tested in a clean disposable profile. Publication, including Unlisted, is Timothy's decision.
3. **The clean-client install** (section 8). This is the later acceptance and release gate.

Collection slug and revision, and saved FOMOD choices, stay **unverified** until they are observed in Vortex 2.7.1. The launcher reads them but never relies on them before that.

## 1. What changes

| Before (PR #6, first cut) | After (this design) |
|---|---|
| The launcher asked the extension to install, enable, disable and deploy each mod (bespoke per-mod installer). | **Vortex installs the Aetherial Dawn collection with its own collection flow.** That covers downloading, FOMOD choices, enabling, rules, load order and deployment. |
| Extension verbs: `manifest`, `status`, `install`, `enable`, `disable`, `deploy`, `rollback`. | The extension is **read-only**: it reports `status` and `collection`, and nothing else. The install verbs are removed in slice 1 (section 7). Their tests stay in git history. |
| The launcher put Nexus mods straight into `Data`. | **Vortex owns every Nexus file.** The launcher owns only its explicit list (section 4). No file in `Data` has two owners. |

These PR #6 parts are kept:
- the pairing token;
- HMAC-signed jobs with a timestamp and a one-use nonce;
- the port file;
- the exact-file membership check;
- approved packages for "Only the server's mods";
- the two named inventories;
- the feed receipt;
- the exporter (report, plan-only pick, local override);
- RAR (#11).

## 2. Owners and branches

| Area | Owner | Where |
|---|---|---|
| Collection membership, pins, FOMOD choices, conflict rules, plugin-order contract, the 22-row delta | Mods chat | ingame-ui draft PR #8 (`collection/…`) |
| Native Vortex bridge, progress UI, Ready/Play gate, launcher-to-collection handoff, migration runbook | Launcher chat | launcher `vortex-collection` (off `vortex-profile`), new draft PR |
| Server contract: RAR, plan-only pick, local override, export receipt | Launcher chat (exporter code) and Mods chat (list) | launcher #11 and #6; ingame-ui #8 |
| Harness scenarios for each UI state; defect register | Launcher QA | launcher QA branches; `launcher-qa/defects.md` |
| Publication, merge, release, cutover, profile migration | Timothy | – |

## 3. Player journey and UI states

The launcher shows one line per step, with exact counts and a per-item error list. Each step is re-checked on every launcher start, after the game exits, and when Vortex reports a deployment.

1. **Skyrim**
   - States: `not found` → `wrong version` → `ok (1.6.1170)`.
   - When it isn't installed, the launcher shows the Steam install steps, then verifies again.
2. **Vortex**
   - States: `not installed` → `older than 2.7.1` → `running without the Aetherial Dawn extension` → `ready`.
   - The extension is installed by the launcher into Vortex's user plugins folder (section 5).
3. **Nexus sign-in**, done in Vortex, never in the launcher.
   - States: `signed out` → `free account` → `Premium`.
   - A free account is a supported path, not an error: the launcher says "Vortex will ask you to press Download for each mod".
4. **Collection** (Aetherial Dawn, the pinned revision)
   - `not added`: the launcher offers a button that opens the collection page with the `nxm://` link. The page stays a local draft until Timothy publishes it.
   - `downloading N of M`, with a free-account line: "N of M waiting for you to press Download in Vortex".
   - `installing N of M`.
   - `needs your choice`: a FOMOD dialog is open in Vortex.
   - `installed N of M · enabled N of M · deployed N of M`.
   - `wrong revision (has R, needs R')`.
   - `ready`.
   - Per-item errors name the mod, its Nexus file id and Vortex's own message.
5. **Profile**
   - States: `no "Aetherial Dawn" profile` → `another profile is active` → `two profiles named Aetherial Dawn` → `ok`.
   - The launcher never switches the profile; it says what to click.
6. **Launcher tools:** SKSE, the SkyMP client, Crash Logger and Souls RE (section 4).
   - States: `missing` → `wrong build` → `ok`.
7. **Server match**
   - The server's running revision receipt must match the collection revision and plugin order the client has.
   - States: `server unreachable` → `server on another revision` → `ok`.
8. **Play** is enabled only when steps 1 to 7 are all `ok`.

## 4. Ownership of files

- **Vortex (the collection):** every Nexus-hosted file, including:
  - the 8 former launcher built-ins, after their handoff;
  - USSEP, once it is decided.
- **The launcher:** the SkyMP client files, SKSE, Crash Logger, Souls RE, `plugins.txt` alias copies, and its own ledger.
  - The list is explicit and published in the manifest (`launcherOwned`).
- **Handoff of a launcher built-in to the collection**, per file, and reversible. The states are `launcher-owned` → `handoff-pending` → `collection-owned`:
  1. Vortex deploys the collection's copy.
  2. The launcher checks that Vortex's deployment names that file for that mod and file id.
  3. The launcher sets its own copy aside (never deleting it) and records this in its ledger.
  4. Rollback puts the copy back.
- **Until a file is `collection-owned`, it is not in the collection's install set for players.**

## 5. Extension install and pairing (Codex 5876119420 / 5875974143)

- The launcher copies the extension into `%APPDATA%\Vortex\plugins\aetherial-dawn\` **(verify)**. The copy goes into a versioned temp folder and is renamed into place. Unknown files there are preserved.
  - Built (`vortex::install_extension`): only when the player presses **Connect Vortex** in the mods window, which shows when Vortex manages the game and the launcher isn't paired yet.
  - The launcher carries the extension's files. Each file is written in full to `%APPDATA%\Vortex\aetherial-dawn-extension.staging`, then renamed over the old one. `info.json` goes last, so a stopped install is finished by the next one.
  - If the files are identical, nothing is written. A later version (by `info.json`) left by a newer launcher is kept. With no `%APPDATA%\Vortex` folder, nothing is written.
  - Afterwards the player is told to restart Vortex once. The command `vortex_connect { fresh: true }` is the re-pair; it has no button yet.
  - Manual ZIP install stays documented as a recovery route.
- Pairing uses a persistent 32-byte token in `%LOCALAPPDATA%\gg.aetherialdawn.launcher\vortex\token`.
  - It is rotated only on an explicit re-pair or a security failure.
  - Vortex needs a restart to read a new token.
- Every request is HMAC-SHA256 signed (`x-ad-sig`), with a ±60 s timestamp and a one-use nonce.
  - The same test vector passes in Rust (`core/src/vortex.rs`) and in `jobs.js`.

## 6. Collection manifest and receipts

### 6.1 `aetherial-collection.json` (served next to `mods.json`; list sha256 recorded like the feed receipt)

```json
{
  "schema": 1,
  "game": "skyrimspecialedition",
  "collection": { "slug": "<slug>", "revision": 1, "sha256": "<sha256 of the revision's collection.json>" },
  "profileName": "Aetherial Dawn",
  "mods": [
    { "id": "ussep", "modId": 266, "fileId": 733846, "owner": "collection",
      "plugins": ["Unofficial Skyrim Special Edition Patch.esp"], "fomod": [], "state": "collection-owned" }
  ],
  "launcherOwned": ["Data/SKSE/Plugins/CrashLogger.dll", "skse64_loader.exe"],
  "pluginOrder": { "sha256": "<sha256 of the order contract>", "count": 70 },
  "server": { "listHash": "<serverlane list_hash>", "revision": "<world revision>" }
}
```

Rules:
- `mods` must equal the collection revision's required rows exactly, by `(modId, fileId)`. The launcher refuses to show Ready on a mismatch.
- `fomod` holds the choices the collection carries. They are checked against Vortex's stored installer choices **(verify)**.
- `state` follows section 4.

### 6.2 Client readiness receipt (written by the launcher at Ready, local, shown in Copy diagnostics)

```json
{ "collection": {"slug": "…", "revision": 1}, "manifestSha256": "…",
  "profile": {"id": "…", "name": "Aetherial Dawn"},
  "mods": {"required": 86, "installed": 86, "enabled": 86, "deployed": 86},
  "pluginsTxtSha256": "…", "orderSha256": "…", "launcherVersion": "0.1.x",
  "checkedAt": "2026-…Z" }
```

### 6.3 Server receipt

This is Package B, owned by the Mods chat and the exporter.
- `export.json` already records:
  - the list hash;
  - the source (served, or local override with its sha256);
  - the zip sha256;
  - each plugin's sha256.
- The instance-bound running-server receipt is the server side's work. The launcher reads it for step 7.

## 7. First reviewable slice (launcher `vortex-collection`, draft)

1. **Extension becomes read-only.**
   - `status` returns:
     - the active profile;
     - the profiles named Aetherial Dawn;
     - per mod: id, state, `modId`, `fileId`, enabled, and `installerChoices` when present **(verify)**;
     - the collection mods, meaning type `collection`, with slug and revision attributes **(verify)**;
     - the last deployment time.
   - `install`, `enable`, `disable`, `deploy` and `rollback` are removed.
   - Tests use fixture Vortex state for every state in section 3, steps 4 and 5.
2. **Launcher bridge.**
   - It installs the extension into Vortex's plugins folder, reads status through `vortex::call`, and turns it into the counts and states of section 3 (a pure function with a fixture test per state).
   - `vortex::membership` checks the manifest's `(modId, fileId)` rows.
   - `vortex::approved` feeds "Only the server's mods".
3. **UI:** the Requirements window shows the step lines and counts. Fast-Play harness scenarios cover:
   - not paired;
   - Vortex closed;
   - wrong profile;
   - downloading, with a free account;
   - FOMOD pending;
   - all deployed.
4. **Play gate:** in the backend, not the UI. It refuses Play unless steps 1 to 7 are `ok`, and re-checks after the game exits.
   - For milestone 1, the manifest has no `collection`. The gate then checks only the required `(modId, fileId)` rows in the active profile. A test (`the_existing_11_row_profile_reads_as_incomplete_not_ready`) proves that an 11-row profile reads as incomplete, and names what is missing and any other version still switched on.
5. **Out of slice 1:**
   - the handoff of built-ins;
   - migration of Timothy's profile;
   - the server-match step, which waits on the server receipt.

## 8. Clean-room test script (evidence, not CI)

**Setup:** a new local Windows account or VM, and legitimate Steam Skyrim SE. There is no Vortex, no launcher and nothing in `Data`. Use a **free** Nexus account.

1. Install the test launcher (an unsigned test build, never main). Record the version and sha256.
2. Follow the launcher's lines only. Record each state and count shown, with screenshots.
3. **Interrupt.** Close Vortex during downloads, and reboot during install. The launcher must show the right counts after each, and resume.
4. **Wrong profile.** Switch Vortex to Default. The launcher must say so and refuse Play. Switch back: it must be Ready.
5. **Missing file.** Disable one required mod in Vortex. The launcher must name it and refuse Play.
6. **Installer mismatch.** Reinstall one FOMOD mod with another choice. The launcher must name it **(verify that choices are readable)**.
7. **Launch.** Play → main menu → join the test server. Save the log, `plugins.txt` sha256 and the receipt.
8. **Two clients.** Join the same world, see each other, then reconnect.
9. **Premium account:** repeat steps 1 and 2 only, and record the difference in clicks.

**Timothy's own profile migration is a separate run:**
- First, a backup of his Vortex state through Vortex's own backup, plus a copy of his `Data`.
- Then the handoff (section 4).
- Rollback is proven before any real use.

## 9. Open questions for Codex

1. The extension install path, and whether Vortex 2.7.1 loads an unsigned user extension without a prompt **(verify on the disposable profile)**.
2. Whether the collection install dialog can target the existing "Aetherial Dawn" profile, or always makes its own. The addendum says the archived source allows an existing same-game profile; this is to be proven.
3. Where Vortex 2.7.1 keeps a mod's FOMOD choices in state, so the launcher can check them read-only.
