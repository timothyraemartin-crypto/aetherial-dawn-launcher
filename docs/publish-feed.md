# Publishing the feed (sign and pin) in one command

On the server (or anywhere with the web root and the key):

    sudo AD_WEB_ROOT=/srv/aetherial-dawn/launcher AD_FEED_KEY=/etc/aetherial-dawn/skymp-sign.pem scripts/publish-feed.sh

It (1) finds every direct-download entry in `mods.json` without a `sha256`, hashes the file (from the
web root when the address is ours, else by downloading it) and pins it in a temp copy;
(2) fetches `sign-feed` from the newest release (checked against its `.sha256`); (3) signs
`mods.json` and `client/manifest.json` in that copy and verifies it; (4) only then swaps the files and their signatures in together, keeping the replaced files in `AD_BACKUP_DIR` (outside the web root). A failure at any step leaves the live files untouched, or restores them. `--check` only reports and changes nothing.
The two paths above are the defaults and are guesses: set the real ones.

Before the launcher release that first expects pins and signatures (0.1.106) the newest release has no
`sign-feed` yet, so run `scripts/publish-feed.sh --check` (reports entries whose `sha256` is not 64 hex characters, or that lack
`https`, which the launcher drops) and then `--pin-only` (fills missing or invalid `sha256`, signs nothing, needs
no key or tool; refuses when a `.sig` exists). After the release, run it with no flag to sign.

Or from GitHub (choose mode check, pin-only or publish): Actions > Publish launcher feed > Run workflow (needs secrets `AD_SSH_HOST`,
`AD_SSH_USER`, `AD_SSH_KEY`, `AD_SSH_KNOWN_HOSTS`; optional variables `AD_WEB_ROOT`, `AD_FEED_KEY`).

Run it after every change to `mods.json` or the manifest. The launcher accepts unsigned files until
it has once seen a valid signature, so the order is: release the launcher, publish, then every
later edit is published with this script (see docs/signing-feeds.md, "Rollout order").

## Removing the mod list (after the mod-free launcher is released)

Order matters: old launchers still read `mods.json`, so never remove it before the release is live. Then:

1. Actions > Publish launcher feed > mode `strip-mods-preview`: lists each entry as KEEP or remove and the hosted files it would move; changes nothing.
2. Same workflow, mode `strip-mods`: keeps only the custom (our own) entries named in `AD_KEEP_IDS` (default none; set the repo variable `AD_KEEP_IDS`), removing Nexus and other third-party entries (other keys stay), signs, verifies, swaps in with the old `mods.json` and `.sig` backed up, then moves (never deletes) the hosted mod files into `AD_BACKUP_DIR/files`. `client/`, `masters.json` and files `client/manifest.json` names are not touched.
3. Rollback: copy the backed-up `mods.json`, `.sig` and `files/` back into the web root, then run mode `publish`.
