# Play-testing a mod pack (Nexus collection revision) fast

Why: collection rev 7 (2026-10-13) reached Timothy's PC and failed three ways before he could play: a Vortex re-deploy rewrote plugins.txt (black screen), Actor Limit Fix asked the Address Library for an ID the 1.6.1170 file lacks (crash 3 s in), and JContainers64.dll is not a 1.6.1170 build (SKSE box). Nothing looked at any of it before Play. Labels: CONFIRMED / REPORTED / INFERRED / UNKNOWN.

## Three layers, fastest first

| Layer | Where | Time | Catches | State |
| --- | --- | --- | --- | --- |
| 1. Pre-flight | CI (`Pack pre-flight` workflow) or any PC (`pack-preflight scan DIR`) | minutes, no game | DLL that SKSE 2.2.6 refuses on 1.6.1170; DLL the ledger knows crashes; DLL needing Address Library with no `versionlib-1-6-1170-0.bin`; plugin with a missing master; master after its plugin; > 254 full plugins; unusable plugin; every DLL never seen loading is listed UNVERIFIED | built, offline-tested |
| 2. Trial launch | one PC, one Play | ~2 min | what only the real game shows: Address Library IDs, load crashes, SKSE boxes, black screens | `pack-preflight triage` built; the one-click trial in the launcher is NOT built (see Next) |
| 3. Server join | staging server 7790 + a tester | ~5 min | server-side load order, desync, join failures | exists on the VPS (UNKNOWN, cloud cannot reach it); not wired to packs |

A pack runs layer 1 on every revision, then layer 2 on one PC for anything layer 1 marks FAIL or UNVERIFIED, then layer 3. Layer 1 cannot say a DLL runs; it can say it will not, and which DLLs to test first.

## The ledger (what makes layer 1 improve)
`core/data/dll-ledger.json` records what each SKSE DLL did on 1.6.1170 in a real game: `crashes`, `refused` or `loads`, with evidence. A DLL name not in it is UNVERIFIED (WARN). After any trial launch:

    pack-preflight triage "<Skyrim>/../Documents/My Games/Skyrim Special Edition/SKSE/skse64.log"

prints ledger lines for each DLL SKSE loaded, refused or that hit "Identifier not found, N". Paste them into the ledger in a PR. Seeded today: ActorLimitFix.dll crashes (CONFIRMED skse64.log), JContainers64.dll refused (REPORTED), CustomSkills.dll loads (REPORTED, works in game UNKNOWN). Matching is by file name, or by sha256 when an entry gives one.

## Running it
- CI: Actions > **Pack pre-flight** > Run workflow (revision empty = the one in `server/launcher-rules.json`; `strict` fails the job on a FAIL). It builds `pack-preflight`, runs `scripts/pack-fetch.sh` (downloads only DLLs, plugins and the Address Library file from each mod; skips archives over 300 MB and says "NOT CHECKED"), scans against the collection's plugin order and the public `masters.json`, and writes the report to the run summary and an artifact. First run (PR 192, rev 7, CONFIRMED in the job log): it works end to end; 19 mod archives were read, 7 downloads failed (a link with spaces, fixed) and 2 archives over 300 MB were skipped, all named NOT CHECKED in the report.
- PC: `pack-preflight scan "A:\steam\steamapps\common\Skyrim Special Edition\Data" --rules server/launcher-rules.json --masters masters.json`.
- `pack-preflight addrlib versionlib-1-6-1170-0.bin 523948` says whether an ID is in an Address Library file (the missing ID of the Actor Limit Fix crash). The real 1.6.1170 file is format 2, which the reader does NOT yet read (CONFIRMED by the first run), so `addrlib` is unusable on it until a format 2 reader is written.

## Limits
- Archives with a FOMOD installer hold several variants of one DLL; which one is installed is the installer's choice. When one name has different builds, a bad one is only a WARN.
- "Fits SKSE" is not "works": ActorLimitFix passes SKSE's own version test and still crashes. That is why the ledger exists.
- Nothing here blocks Play. Every check is a report; the launcher is unchanged.
- Kill switches: nothing to switch off in the launcher (no launcher change); the workflow is manual (or on PRs touching it).

## Next (not built, defaults chosen)
1. **Trial launch button** (launcher, warn-only): Play with a "trial" flag that, 90 s after the game starts, reads skse64.log, runs `triage`, and shows "these DLLs loaded / these did not" plus the ledger lines. Needs a Windows build and a PC check.
2. **Pre-flight at Play**: run layer 1 on the installed Data folder and show a health row (`pack`), ledger from the signed rules feed. Needs `health.rs` plus a rules field and a PC check.
3. **Staging join**: after layers 1-2 pass, a tester joins staging (7790) with the new pack; the headless bot checks the server's load order. The launcher's server address is a build-time setting (`AD_BASE_URL`), so a staging build is a separate tagged build; recommended only if layer 2 passes and the server side changed.
