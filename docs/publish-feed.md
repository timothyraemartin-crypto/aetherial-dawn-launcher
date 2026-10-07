# Publishing the feed (sign and pin) in one command

On the server (or anywhere with the web root and the key):

    sudo AD_WEB_ROOT=/srv/aetherial-dawn/launcher AD_FEED_KEY=/etc/aetherial-dawn/feed-signing.pem scripts/publish-feed.sh

It (1) finds every direct-download entry in `mods.json` without a `sha256`, hashes the file (from the
web root when the address is ours, else by downloading it) and pins it in a temp copy;
(2) fetches `sign-feed` from the newest release (checked against its `.sha256`); (3) signs
`mods.json` and `client/manifest.json` in that copy and verifies it; (4) only then swaps the files and their signatures in together, keeping the replaced files in `AD_BACKUP_DIR` (outside the web root). A failure at any step leaves the live files untouched, or restores them. `--check` only reports and changes nothing.
The two paths above are the defaults and are guesses: set the real ones.

Or from GitHub: Actions > Publish launcher feed > Run workflow (needs secrets `AD_SSH_HOST`,
`AD_SSH_USER`, `AD_SSH_KEY`, `AD_SSH_KNOWN_HOSTS`; optional variables `AD_WEB_ROOT`, `AD_FEED_KEY`).

Run it after every change to `mods.json` or the manifest. The launcher accepts unsigned files until
it has once seen a valid signature, so the order is: release the launcher, publish, then every
later edit is published with this script (see docs/signing-feeds.md, "Rollout order").
