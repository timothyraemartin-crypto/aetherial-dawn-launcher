#!/usr/bin/env bash
# One command to publish the launcher feed: fill missing sha256 pins, sign, verify.
# Run on the server (or anywhere that has the web root and the key):
#   sudo scripts/publish-feed.sh                  fill hashes, sign, verify
#   sudo scripts/publish-feed.sh --check          only report; changes nothing
# Needs bash, jq, curl, sha256sum. Fetches the sign-feed tool from the newest GitHub release
# (checked against its .sha256) unless SIGN_FEED points at one.
# Settings (env): AD_WEB_ROOT   folder served at https://vps-d38c928e.vps.ovh.us/launcher
#                 AD_FEED_KEY   Ed25519 PKCS#8 PEM private key (never in a repo)
#                 AD_BASE_URL   public address of that folder (to map url entries to local files)
# See docs/signing-feeds.md.
set -euo pipefail

WEB_ROOT=${AD_WEB_ROOT:-/srv/aetherial-dawn/launcher}
KEY=${AD_FEED_KEY:-/etc/aetherial-dawn/feed-signing.pem}
BASE_URL=${AD_BASE_URL:-https://vps-d38c928e.vps.ovh.us/launcher}
REPO=${AD_RELEASE_REPO:-timothyraemartin-crypto/aetherial-dawn-launcher}
CHECK=0; [ "${1:-}" = "--check" ] && CHECK=1

for t in jq curl sha256sum; do command -v "$t" >/dev/null || { echo "$t is needed"; exit 1; }; done
MODS="$WEB_ROOT/mods.json"
[ -f "$MODS" ] || { echo "no $MODS (set AD_WEB_ROOT)"; exit 1; }
jq -e '.mods | type == "array"' "$MODS" >/dev/null || { echo "$MODS is not a launcher mod list"; exit 1; }

# 1. Every direct download (a url and no nexus) must pin a sha256, or the launcher drops it.
missing=$(jq -r '.mods[] | select(.url != null and .nexus == null and (.sha256 // "") == "") | .id' "$MODS")
if [ -n "$missing" ]; then
  echo "Entries without sha256: $(echo $missing | tr '\n' ' ')"
  [ "$CHECK" = 1 ] && exit 1
  cp -p "$MODS" "$MODS.bak"
  tmp=$(mktemp); cp "$MODS" "$tmp"
  tmpdl=$(mktemp)
  trap 'rm -f "$tmp" "$tmpdl"' EXIT
  for id in $missing; do
    url=$(jq -r --arg id "$id" '.mods[] | select(.id == $id) | .url' "$MODS")
    if [[ "$url" == "$BASE_URL"/* && -f "$WEB_ROOT/${url#"$BASE_URL"/}" ]]; then
      sum=$(sha256sum "$WEB_ROOT/${url#"$BASE_URL"/}" | cut -d' ' -f1)
    else
      curl -fsSL --max-time 600 -o "$tmpdl" "$url" || { echo "can't download $url for $id"; exit 1; }
      sum=$(sha256sum "$tmpdl" | cut -d' ' -f1)
    fi
    echo "  $id  $sum"
    jq --arg id "$id" --arg s "$sum" '(.mods[] | select(.id == $id)) .sha256 = $s' "$tmp" > "$tmp.new" && mv "$tmp.new" "$tmp"
  done
  # Replace the file in one step so a launcher never reads half of it.
  cat "$tmp" > "$MODS.new" && mv "$MODS.new" "$MODS"
  echo "Pinned the hashes above in $MODS (old copy: $MODS.bak). Check the files are the ones you meant."
fi

# 2. Sign and verify.
tool=${SIGN_FEED:-$(command -v sign-feed || true)}
if [ -z "$tool" ]; then
  tool=$(mktemp -d)/sign-feed
  rel="https://github.com/$REPO/releases/latest/download"
  curl -fsSL -o "$tool" "$rel/sign-feed-linux-x86_64"
  want=$(curl -fsSL "$rel/sign-feed-linux-x86_64.sha256" | cut -d' ' -f1)
  [ "$(sha256sum "$tool" | cut -d' ' -f1)" = "$want" ] || { echo "sign-feed download does not match its .sha256"; exit 1; }
  chmod +x "$tool"
fi
if [ "$CHECK" = 1 ]; then "$tool" verify "$WEB_ROOT"; exit; fi
[ -f "$KEY" ] || { echo "no signing key at $KEY (set AD_FEED_KEY)"; exit 1; }
"$tool" sign "$KEY" "$WEB_ROOT"
"$tool" verify "$WEB_ROOT"
echo "Feed published and verified."
