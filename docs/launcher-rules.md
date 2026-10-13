# launcher-rules.json: fixing Play-time checks without a launcher release

The launcher fetches `launcher-rules.json` from the server (next to `mods.json`)
once per Play and applies it right before the checks it controls. Editing the
file is the fix; nothing is built, tagged or released.

## Where it comes from

1. The server copy, checked against the pinned key (`launcher-rules.json.sig`,
   same rule as `mods.json`: once a launcher has seen a signature it refuses the
   file without one). The verified bytes are saved as `launcher-rules.json` in the
   launcher's data folder.
2. The saved copy, when the server can't be reached or the file is refused.
3. Built-in defaults (today's behaviour) when there is neither, or the server
   publishes no file (a withdrawn file also deletes the saved copy).

`launcher.log` says which one applied: `rules: launcher rules <rev> from the server | the saved copy | the built-in defaults`.

## Fields (all optional; a missing or wrongly typed field keeps its default)

| Field | Values | Effect |
| --- | --- | --- |
| `schema` | `1` | A newer schema is refused (saved copy / defaults are used). |
| `rev` | text | Shown in the log. Change it with every edit. |
| `off` | `true` | Kill switch: everything below is ignored. |
| `checks.firewallFix`, `udpProbe`, `connectWatch`, `platformGuard`, `sessionUpload`, `archiveGate` (off unless true) | `false` | Switches that part off. `status.json` `launcher.*` still works; a part runs only if both allow it. |
| `checks.pingStats` | `false` | Skips the 10-ping burst after the UDP probe (round trip, loss, jitter; shown as "Connection quality", Warn from 5 % loss, 150 ms average or 100 ms spread; Play never stops). |
| `tune.on`, `tune.settings` | list | Ini limits applied each Play to `Skyrim.ini` / `SkyrimPrefs.ini` in Documents. Entry: `file`, `section`, `key`, `mode` (`set`, `max` = only lower, `min` = only raise), `value`, `on` (`false` disables that entry; `tune.on:false` all). Only keys in `gameini::TUNABLE` (fShadowDistance, iShadowMapResolution, fPostLoadUpdateTimeMS, uGridsToLoad) and in range are accepted, everything else is ignored. A `max`/`min` entry never adds a missing line. The first original is kept as `<ini>.aetherial-dawn-backup`; each change is logged as `play: tuned <file>: key: old -> new`. The shipped entries are all `"on": false` until staff turn one on. |
| `checks.serverOrder` | `"fail"` (default), `"warn"`, `"off"` | What a plugin-order mismatch does: stop Play, show a warning and go on, or say nothing. |
| `order.ignore` | plugin file names | The order check skips findings that name these (e.g. a Creation Club master that shifts a position). |
| `order.emptyCcc` | `false` | Don't empty `Skyrim.ccc` for the session. |
| `order.rewriteLoadorderTxt` | `false` | Leave `loadorder.txt` alone. |
| `allow.files` | `Data/Foo.esp` ... | Game-folder paths the no-mods check leaves in place. Relative, no `..`, no backslash. |
| `text.orderMismatch` | text, <= 400 chars | Replaces the "plugins don't match" message; `{which}` and `{tell}` are filled in. Any new wording needs Timothy's OK first. |

The expected plugin order itself stays in `masters.json` (also fetched each Play);
this file changes what the launcher does about a mismatch, not the order.

It cannot download, run or delete anything. Text is stripped of control characters.

## Publishing a change (no launcher release)

1. Edit `server/launcher-rules.json` in a PR (bump `rev`), merge.
2. Run the **Publish launcher feed** workflow with mode `rules`. It runs
   `scripts/publish-feed.sh --rules <file>` on the server: checks the file, signs
   it with the feed key, swaps it in with the other feeds and verifies, keeping
   the old files in the backup folder. Players pick it up on their next Play.
3. To undo: publish the previous version, or set `"off": true`.

Needs a launcher that contains `core/src/rules.rs` (the release after this
change) and a `sign-feed` from that release; the script refuses to publish
unsigned rules.

## `collection` (required Nexus collection)

The required mod list is the Nexus collection "Aetherial Dawn" (slug `nzwgrw`, revision 1). The launcher does not download or install it; Play adds a health row "Required mods (Nexus collection)" that lists mods that are missing and plugins that are not switched on or out of order. Default level `warn`: Play goes on. `"level": "block"` stops Play (only when Timothy asks for it by name), `"off"` hides the row.

- `mods[]`: `name`, `modId`, `fileId`, `version` (from the Nexus API) and `files`, game-relative paths that must all exist (INFERRED from each mod's usual files; neither the API nor collection.json lists them, and Nexus files carry no version marker, so versions are shown but not compared).
- `pluginOrder`: the collection's plugins in load order, checked against the active lines of plugins.txt. A name with spaces ("Unofficial Skyrim Special Edition Patch.esp") also matches its dash-joined alias (aliases.rs), the name the launcher actually runs.
- New revision: `scripts/collection-rules.sh [REVISION]`, review the diff, `publish-feed.sh --rules server/launcher-rules.json`. The built-in copy is `core/data/collection-nzwgrw-r1.json` (used until a file is fetched); update it and the `include_str!` name in `core/src/collection.rs` at the next launcher release.

The no-mods guard (modguard.rs) no longer sets the collection's files aside: `Collection::owns` exempts its plugins (and alias names), archives and inis named like them, detection files and folders, and each mod's `allow` prefixes (e.g. SkyUI `Data/Interface`; INFERRED). `NEXUS_API_KEY` (Actions secret) lets the "Refresh Nexus collection list" workflow read the real plugin order from the collection's own `collection.json`; CONFIRMED for revision 1: unofficial skyrim special edition patch.esp, then SkyUI_SE.esp.
