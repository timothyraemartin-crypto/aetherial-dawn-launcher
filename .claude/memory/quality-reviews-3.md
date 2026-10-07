---
name: quality-reviews-3
description: Open findings from launcher quality reviews, part 3 (mod list, Vortex bridge, game-folder tidying, game build)
type: reference
verified: 2026-10-07
refs: core/src/modlist.rs, core/src/vortex.rs, core/src/loadorder.rs, core/src/aliases.rs, core/src/downgrade.rs, core/src/patcher.rs
---
Read-only reviews on 2026-10-07; no code changed. Continues quality-reviews-2.

**Mod list and installers** (solid; 279 core tests pass; https://claude.ai/artifact/6f9FgQfwBuaYiWqP6cMDy5)
- FIX IN DRAFT PR #58 (not merged): `mods.json` signed; direct downloads must pin a sha256 (topic signed-feeds). Was: unsigned, hash-checked only if the list gives a sha256 (`modlist.rs`, `src-tauri/src/mods.rs`).
- Pandora is documented but cannot run (`tools.rs`); a single removed mod is never cleaned up; `modlist.rs` is ~2,700 lines and not rustfmt-formatted; the `mods.json` fetch path is untested.

**Vortex bridge** (signing/replay correct; 45 tests pass; https://claude.ai/artifact/GVXYTmgVA9PuDmzpXYQz51)
- Replies are unauthenticated and the extension never deletes its port file, so another local process could fake Ready (`vortex.rs`). Re-pair while Vortex is open shows a misleading "open Vortex" message. The gate fails open if `aetherial-collection.json` cannot be fetched, (fetched once per Play after draft PR #55). DECIDED 2026-10-07 (Timothy): the Vortex check stays fail-open when the file cannot be fetched, as intended.
- Low: token permissions assumed, 600 s timeout for rejected verbs, hand-written HMAC, hard-coded mod ids, ~1,645-line file.

**Game folder tidying** (https://claude.ai/artifact/QbzAdcRpYQa77b1fS5dQ6y)
- CONFIRMED data loss (reproduced with a temp test; fix in draft PR #54, not merged): `force_on` and `switch_on` in `loadorder.rs` turn a failed ANSI plugins.txt read into empty text and rewrite the file with no backup. `force_on` runs on every Play.
- `aliases.rs` deleted an existing dash-name file without checking the launcher made it (fixed in #54, with atomic writes and a second backup). #54 also touches `gameini.rs` (Skyrim.ini writes; check it covers the non-atomic-write finding). The `allowlist.rs` doc comment contradicts the 2026-09-26 sweep decision.
- Good: tidying moves rather than deletes; `retire_unlisted` refuses lists that drop more than half the mods.

**Game build detection** (good on the shipped patch route; 44 tests pass; https://claude.ai/artifact/VvZhT5jGC2fvX3Ad5C5NBm)
- The Steam downgrader (`downgrade.rs`, `steamapp.rs`) was NOT called by the app (`ui/app.js`); draft PR #60 removes it.
- MulderLoad gaps: later parts unchecked if the hash pins only part 1; a damaged part is kept after an unpack error; no disk-space check; only the exe version is confirmed after patching. Three disagreeing lists of Steam's own files (`version.rs`, `patcher.rs`, `pristine.rs`).
