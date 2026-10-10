#!/usr/bin/env bash
# One command to publish the launcher feed: fill missing sha256 pins, sign, verify.
# Run on the server (or anywhere that has the web root and the key):
#   sudo scripts/publish-feed.sh                  fill hashes, sign, verify
#   sudo scripts/publish-feed.sh --check          only report; changes nothing
#   sudo scripts/publish-feed.sh --pin-only       BEFORE a launcher release: fill missing sha256 pins in mods.json,
#                                                 no signing (needs no sign-feed and no key); refuses a signed list
#   sudo scripts/publish-feed.sh --strip-mods-preview   list what --strip-mods would do; changes nothing
#   sudo scripts/publish-feed.sh --strip-mods     AFTER the mod-free launcher is released: set mods.json to only the custom entries
#                                                 (AD_KEEP_IDS=id1,id2; default none; keeps its other keys), sign, verify, swap, then MOVE (never delete) the mod
#                                                 files it pointed at into AD_BACKUP_DIR/files. client/ and masters.json stay.
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
KEY=${AD_FEED_KEY:-/etc/aetherial-dawn/skymp-sign.pem}
BASE_URL=${AD_BASE_URL:-https://vps-d38c928e.vps.ovh.us/launcher}
REPO=${AD_RELEASE_REPO:-timothyraemartin-crypto/aetherial-dawn-launcher}
BACKUP_DIR=${AD_BACKUP_DIR:-/var/backups/aetherial-deploy/feed-$(date +%Y%m%d%H%M%S)}
RELEASE_BASE=${AD_RELEASE_BASE:-https://github.com/$REPO/releases/latest/download}
CHECK=0; PIN_ONLY=0; STRIP=0; PREVIEW=0
case "${1:-}" in --check) CHECK=1 ;; --pin-only) PIN_ONLY=1 ;; --strip-mods) STRIP=1 ;; --strip-mods-preview) STRIP=1; PREVIEW=1 ;; "") ;; *) echo "unknown option ${1}"; exit 1 ;; esac
FEEDS=(mods.json client/manifest.json)

for t in jq curl sha256sum; do command -v "$t" >/dev/null || { echo "$t is needed"; exit 1; }; done
MODS="$WEB_ROOT/mods.json"
[ -f "$MODS" ] || { echo "no $MODS (set AD_WEB_ROOT)"; exit 1; }
jq -e '.mods | type == "array"' "$MODS" >/dev/null || { echo "$MODS is not a launcher mod list"; exit 1; }

WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT

# 1. Preflight, before anything is touched. The launcher drops a direct download that has no full sha256
# or isn't https, so those are found first and named; they are what makes players lose mods.
UNPINNED='.mods[] | select(.url != null and .nexus == null and ((.sha256 // "") | tostring | test("^[0-9a-fA-F]{64}$") | not)) | .id'
PLAIN='.mods[] | select(.url != null and (.url | startswith("https://") | not)) | .id'
plain=$([ "$STRIP" = 1 ] || jq -r "$PLAIN" "$MODS")
[ -z "$plain" ] || { echo "Entries whose address is not https (the launcher drops them): $(echo $plain | tr '\n' ' ')"; exit 1; }
missing=$([ "$STRIP" = 1 ] || jq -r "$UNPINNED" "$MODS")
if [ "$CHECK" = 1 ] && [ -n "$missing" ]; then echo "Entries without sha256: $(echo $missing | tr '\n' ' ')"; exit 1; fi
# The launcher also drops any entry that names a single `file` to check or copy unless it carries a full
# 64-character sha256 (core/src/modlist.rs). A nexus entry is not covered by the direct-download check
# above, so it would vanish from every player's list without a word.
NOFILEPIN='.mods[] | select(.file != null and ((.sha256 // "") | tostring | length) != 64) | .id'
nofilepin=$([ "$STRIP" = 1 ] || jq -r "$NOFILEPIN" "$MODS")
if [ "$CHECK" = 1 ] && [ -n "$nofilepin" ]; then echo "Entries with a file but no full sha256 (the launcher drops them): $(echo $nofilepin | tr '\n' ' ')"; exit 1; fi
# Strip mode: hosted mod files to move after the swap = files under our address named by the old list,
# minus anything under client/ (SkyMP file sync) and anything the client manifest mentions.
MOVE=()
# Custom (our own) entries survive: AD_KEEP_IDS=id1,id2. Everything else (Nexus, third party) goes.
KEEP_JSON=$(jq -n --arg k "${AD_KEEP_IDS:-}" '$k | split(",") | map(select(length>0))')
if [ "$STRIP" = 1 ]; then
  echo "Keeping ids: $KEEP_JSON. Entries:"
  jq -r --argjson keep "$KEEP_JSON" '.mods[] | "  \(if (.id as $i | $keep | index($i)) then "KEEP  " else "remove" end) \(.id)  \(.url // (.nexus|tostring))"' "$MODS"
  while IFS= read -r u; do
    [[ "$u" == "$BASE_URL"/* ]] || continue
    rel=${u#"$BASE_URL"/}; rel=${rel%%[?#]*}
    case "$rel" in client/*|masters.json*|mods.json*|*..*|"") continue ;; esac
    [ -f "$WEB_ROOT/$rel" ] || continue
    if [ -f "$WEB_ROOT/client/manifest.json" ] && grep -qF -- "$rel" "$WEB_ROOT/client/manifest.json"; then echo "  keep (client manifest names it): $rel"; continue; fi
    MOVE+=("$rel")
  done < <(jq -r --argjson keep "$KEEP_JSON" '.mods[]? | select(.id as $i | $keep | index($i) | not) | (.url // empty), (.files[]? // empty | strings)' "$MODS" | sort -u)
  echo "mods.json: $(jq '.mods | length' "$MODS") entries -> $(jq length <<<"$KEEP_JSON"). Hosted files to move to the backup folder: ${#MOVE[@]}"
  for rel in "${MOVE[@]}"; do echo "  move $rel"; done
  [ "$PREVIEW" = 0 ] || exit 0
fi
if [ "$PIN_ONLY" = 0 ]; then
  tool=${SIGN_FEED:-$(command -v sign-feed || true)}
  if [ -z "$tool" ]; then
    tool=$WORK/sign-feed
    if ! curl -fsSL -o "$tool" "$RELEASE_BASE/sign-feed-linux-x86_64" 2>/dev/null; then
      # Releases before the signing one carry no sign-feed: --check still answered the pin question above.
      [ "$CHECK" = 1 ] && { echo "All entries are pinned and https; signatures not checked (no sign-feed in the newest release yet)"; exit 0; }
      echo "can't download sign-feed"; exit 1
    fi
    want=$(curl -fsSL "$RELEASE_BASE/sign-feed-linux-x86_64.sha256" | cut -d' ' -f1)
    [ "$(sha256sum "$tool" | cut -d' ' -f1)" = "$want" ] || { echo "sign-feed download does not match its .sha256"; exit 1; }
    chmod +x "$tool"
  fi
  if [ "$CHECK" = 1 ]; then "$tool" verify "$WEB_ROOT"; exit; fi
  [ -f "$KEY" ] || { echo "no signing key at $KEY (set AD_FEED_KEY)"; exit 1; }
else
  # A signature over the old bytes would no longer match: a launcher that has seen one then refuses the list.
  for f in "${FEEDS[@]}"; do [ ! -f "$WEB_ROOT/$f.sig" ] || { echo "$f.sig exists; run without --pin-only so the list is pinned and signed together"; exit 1; }; done
fi

# 2. Build the new feed in a temp copy.
STAGE=$WORK/stage; mkdir -p "$STAGE/client"
for f in "${FEEDS[@]}"; do [ -f "$WEB_ROOT/$f" ] && cp "$WEB_ROOT/$f" "$STAGE/$f"; done
if [ "$STRIP" = 1 ]; then
  jq --argjson keep "$KEEP_JSON" '.mods |= map(select(.id as $i | $keep | index($i)))' "$STAGE/mods.json" > "$WORK/m.new" && mv "$WORK/m.new" "$STAGE/mods.json"
fi
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

if [ "$PIN_ONLY" = 1 ]; then
  [ -n "$missing" ] || { echo "Nothing to pin."; exit 0; }
  mkdir -p "$BACKUP_DIR"; cp -p "$MODS" "$BACKUP_DIR/mods.json"
  cp "$STAGE/mods.json" "$MODS.new" && mv "$MODS.new" "$MODS"
  echo "Pinned. Unsigned (sign after the launcher release). Previous file: $BACKUP_DIR"
  exit 0
fi

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
for rel in "${MOVE[@]}"; do
  mkdir -p "$BACKUP_DIR/files/$(dirname "$rel")" && mv "$WEB_ROOT/$rel" "$BACKUP_DIR/files/$rel"
done
echo "Feed published and verified. Previous files: $BACKUP_DIR"
