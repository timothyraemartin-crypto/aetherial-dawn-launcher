# Signing the server's mod list and file list

The launcher acts on two files from the server: `mods.json` (what to download
and install) and `client/manifest.json` (which files to put in the game folder,
and which to remove). Both are signed, so editing them on the web server is not
enough to push code to players.

## What is checked

- Each file has a detached Ed25519 signature next to it: `mods.json.sig` and
  `client/manifest.json.sig` (128 hex characters). It signs the exact bytes
  served, prefixed with the file's name, so a signature for one file is never
  accepted for the other (`core/src/feedsig.rs`).
- The launcher pins the public key (`feedsig::PUBLIC_KEYS`). Today that is the
  server key already in `settings::SERVER_PUBLIC_KEYS` (the one that signs the
  gamemode scripts), so the private key that signs these files is the one
  already on the server. Add a second key to the list to rotate.
- A signature that doesn't match is refused, always. A launcher that has once
  seen a valid signature for a file refuses that file whenever the signature is
  missing, so deleting the `.sig` is not a way round the check. Until then an
  unsigned file is still used (so a launcher released before the server signs
  keeps working); `feedsig::REQUIRE_SIGNED = true` ends that for everyone.
- The server address must be `https` (plain http only to this computer).
- `mods.json`: a download from an address the list gives (not Nexus) must pin a
  full SHA-256, or the entry is dropped. Nexus entries are named by mod and file
  id, and are hash-checked when the list gives a hash.
- `manifest.json`: `remove` deletes only files an earlier file list put there
  (the launcher's hash cache has them) and never Skyrim's own files. Others are
  left alone and named in the launcher log.

## Signing on the server

`sign-feed` is built by CI for Linux (`sign-feed-linux-x86_64` in each release).

    sign-feed sign <private-key.pem> <web root>   # writes mods.json.sig, client/manifest.json.sig
    sign-feed verify <web root>                   # checks them with the key the launcher pins
    sign-feed pubkey <private-key.pem>            # the hex public key, to compare with feedsig.rs

`<web root>` is the folder served at the launcher base address
(`https://vps-d38c928e.vps.ovh.us/launcher`). The key is an Ed25519 PKCS#8 PEM
(`openssl genpkey -algorithm ed25519`) and never goes in a repo. `sign` refuses
a key the launcher doesn't pin, since launchers would refuse what it signed.

**Re-run `sign` after every change** to either file, and `verify` afterwards. A
launcher that has seen a signature stops using a file whose signature no longer
matches (Play waits for a current mod list). It asks twice, three seconds
apart, so replacing the file and its `.sig` together is fine.

## Rollout order

1. Before the launcher release, check the live `mods.json`: every entry with a
   `url` and no `nexus` needs `sha256`. Check the `remove` list only names files
   the launcher itself put there.
2. Release the launcher. Unsigned files still work, so nothing breaks.
3. On the server run `sign-feed sign` and `verify`. Older launchers ignore the
   `.sig` files. Updated launchers start requiring them from their next start.
4. Make `sign-feed sign` part of however `mods.json` and the manifest are
   published.
5. Later, once most players have updated, set `REQUIRE_SIGNED = true` in a
   release. This closes the one remaining gap: a first start on a network that
   hides the `.sig`.

## Not covered

- Replay: an old, correctly signed file can be served again. Files carry
  `revision`/`build`, but the launcher doesn't compare them.
- `masters.json`, `server-lane.json`, `aetherial-collection.json` and the
  patch files are fetched as before.
- Nexus downloads with no `sha256` in the list.
