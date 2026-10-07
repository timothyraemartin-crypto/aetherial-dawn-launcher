#!/usr/bin/env bash
# One command to publish the launcher feed: fill missing sha256 pins, sign, verify.
# Run on the server (or anywhere that has the web root and the key):
#   sudo scripts/publish-feed.sh                  fill hashes, sign, verify
#   sudo scripts/publish-feed.sh --check          only report; changes nothing
# Needs bash, jq, curl, sha256sum. Fetches the sign-feed tool from the newest GitHub release
# (checked against its .sha256) unless SIGN_FEED points at one.
# Everything is built and signed in a temp copy first; the live files change only after the copy
# verifies, and they are swapped in together with their signatures. Any failure leaves them as they were.
# Settings (env): AD_WEB_ROOT   folder served at https://vps-d38c928e.vps.ovh.us/launcher
#                 AD_FEED_KEY   Ed25519 PKCS#8 PEM private key (never in a repo)
#                 AD_BASE_URL   public address of that folder (to map url entries to local files)
#                 AD_BACKUP_DIR where the replaced files are kept (never inside the web root)
# See docs/signing-feeds.md.
set -euo pipefail

WEB_ROOT=${AD_WEB_ROOT:-/srv/aetherial-dawn/launcher}
KEY=${AD_FEED_KEY:-/etc/aetherial-dawn/feed-signing.pem}
BASE_URL=${AD_BASE_URL:-https://vps-d38c928e.vps.ovh.us/launcher}
REPO=${AD_RELEASE_REPO:-timothyraemartin-crypto/aetherial-dawn-launcher}
BACKUP_DIR=${AD_BACKUP_DIR:-/var/backups/aetherial-deploy/feed-$(date +%Y%m%d%H%M%S)}
CHECK=0; [ "${1:-}" = "--check" ] && CHECK=1
FEEDS=(mods.json client/manifest.json)

for t in jq curl sha256sum; do command -v "$t" >/dev/null || { echo "$t is needed"; exit 1; }; done
MODS="$WEB_ROOT/mods.json"
[ -f "$MODS" ] || { echo "no $MODS (set AD_WEB_ROOT)"; exit 1; }
jq -e '.mods | type == "array"' "$MODS" >/dev/null || { echo "$MODS is not a launcher mod list"; exit 1; }

WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT

# 1. Preflight, before anything is touched: the signing tool and (unless --check) the key.
tool=${SIGN_FEED:-$(command -v sign-feed || true)}
if [ -z "$tool" ]; then
  tool=$WORK/sign-feed
  rel="https://github.com/$REPO/releases/latest/download"
  curl -fsSL -o "$tool" "$rel/sign-feed-linux-x86_64" || { echo "can't download sign-feed"; exit 1; }
  want=$(curl -fsSL "$rel/sign-feed-linux-x86_64.sha256" | cut -d' ' -f1)
  [ "$(sha256sum "$tool" | cut -d' ' -f1)" = "$want" ] || { echo "sign-feed download does not match its .sha256"; exit 1; }
  chmod +x "$tool"
fi
if [ "$CHECK" = 1 ]; then
  missing=$(jq -r '.mods[] | select(.url != null and .nexus == null and (.sha256 // "") == "") | .id' "$MODS")
  [ -z "$missing" ] || { echo "Entries without sha256: $(echo $missing | tr '\n' ' ')"; exit 1; }
  "$tool" verify "$WEB_ROOT"; exit
fi
[ -f "$KEY" ] || { echo "no signing key at $KEY (set AD_FEED_KEY)"; exit 1; }

# 2. Build the new feed in a temp copy.
STAGE=$WORK/stage; mkdir -p "$STAGE/client"
for f in "${FEEDS[@]}"; do [ -f "$WEB_ROOT/$f" ] && cp "$WEB_ROOT/$f" "$STAGE/$f"; done
missing=$(jq -r '.mods[] | select(.url != null and .nexus == null and (.sha256 // "") == "") | .id' "$MODS")
for id in $missing; do
  url=$(jq -r --arg id "$id" '.mods[] | select(.id == $id) | .url' "$MODS")
  if [[ "$url" == "$BASE_URL"/* && -f "$WEB_ROOT/${url#"$BASE_URL"/}" ]]; then
    sum=$(sha256sum "$WEB_ROOT/${url#"$BASE_URL"/}" | cut -d' ' -f1)
  else
    curl -fsSL --max-time 600 -o "$WORK/dl" "$url" || { echo "can't download $url for $id"; exit 1; }
    sum=$(sha256sum "$WORK/dl" | cut -d' ' -f1)
  fi
  echo "  pin $id  $sum"
  jq --arg id "$id" --arg s "$sum" '(.mods[] | select(.id == $id)) .sha256 = $s' "$STAGE/mods.json" > "$WORK/m.new"
  mv "$WORK/m.new" "$STAGE/mods.json"
done

# 3. Sign and verify the copy. Nothing live has changed yet.
"$tool" sign "$KEY" "$STAGE"
"$tool" verify "$STAGE"

# 4. Swap in. Backups go outside the web root; any failure from here restores them.
mkdir -p "$BACKUP_DIR/client"
for f in "${FEEDS[@]}"; do for e in "" .sig; do [ -f "$WEB_ROOT/$f$e" ] && cp -p "$WEB_ROOT/$f$e" "$BACKUP_DIR/$f$e"; done; done
restore() {
  echo "swap failed, restoring the previous files from $BACKUP_DIR"
  for f in "${FEEDS[@]}"; do for e in "" .sig; do [ -f "$BACKUP_DIR/$f$e" ] && cp -p "$BACKUP_DIR/$f$e" "$WEB_ROOT/$f$e"; done; done
}
trap 'restore; rm -rf "$WORK"' ERR
for f in "${FEEDS[@]}"; do
  [ -f "$STAGE/$f" ] || continue
  # file then signature back to back; a launcher that sees a mismatch retries after 3 seconds
  if ! cmp -s "$STAGE/$f" "$WEB_ROOT/$f"; then cp "$STAGE/$f" "$WEB_ROOT/$f.new"; fi
  cp "$STAGE/$f.sig" "$WEB_ROOT/$f.sig.new"
  [ -f "$WEB_ROOT/$f.new" ] && mv "$WEB_ROOT/$f.new" "$WEB_ROOT/$f"
  mv "$WEB_ROOT/$f.sig.new" "$WEB_ROOT/$f.sig"
done
"$tool" verify "$WEB_ROOT"
trap 'rm -rf "$WORK"' EXIT; trap - ERR
echo "Feed published and verified. Previous files: $BACKUP_DIR"
