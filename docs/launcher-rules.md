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
| `checks.firewallFix`, `udpProbe`, `connectWatch`, `platformGuard`, `sessionUpload` | `false` | Switches that part off. `status.json` `launcher.*` still works; a part runs only if both allow it. |
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
