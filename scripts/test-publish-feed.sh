#!/usr/bin/env bash
# Offline test of scripts/publish-feed.sh with a stand-in sign-feed (sign writes <file>.sig, verify checks it).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fresh() {
  rm -rf "$T/root" "$T/bk"; mkdir -p "$T/root/patches" "$T/root/client"; echo hello > "$T/root/patches/a.ini"; : > "$T/key.pem"
  cat > "$T/root/mods.json" <<JSON
{"mods":[
 {"id":"local","name":"L","url":"https://example.test/launcher/patches/a.ini"},
 {"id":"pinned","name":"P","url":"https://x.test/p.zip","sha256":"abc"},
 {"id":"nx","name":"N","nexus":{"mod":1}}]}
JSON
  echo '{"files":[]}' > "$T/root/client/manifest.json"
  for f in mods.json client/manifest.json; do echo "oldsig" > "$T/root/$f.sig"; done
}
cat > "$T/sign-feed" <<'SH'
#!/bin/sh
# stand-in: "sign" fails when FAIL_SIGN is set; the signature is the file's sha256
root=$3; [ "$1" = verify ] && root=$2
case "$1" in
  sign) [ -z "${FAIL_SIGN:-}" ] || { echo "sign failed" >&2; exit 1; }
        for f in mods.json client/manifest.json; do [ -f "$root/$f" ] && sha256sum "$root/$f" | cut -d' ' -f1 > "$root/$f.sig"; done ;;
  verify) for f in mods.json client/manifest.json; do [ -f "$root/$f" ] || continue
            [ "$(cat "$root/$f.sig" 2>/dev/null)" = "$(sha256sum "$root/$f" | cut -d' ' -f1)" ] || { echo "BAD $f"; exit 1; }; done ;;
esac
SH
chmod +x "$T/sign-feed"
export AD_WEB_ROOT="$T/root" AD_FEED_KEY="$T/key.pem" AD_BASE_URL=https://example.test/launcher SIGN_FEED="$T/sign-feed" AD_BACKUP_DIR="$T/bk"
snap() { (cd "$T/root" && sha256sum mods.json mods.json.sig client/manifest.json client/manifest.json.sig); }

# --check reports and changes nothing
fresh; before=$(snap)
"$here/publish-feed.sh" --check >/dev/null 2>&1 && { echo "FAIL: --check should flag the unpinned entry"; exit 1; }
[ "$before" = "$(snap)" ] || { echo "FAIL: --check changed files"; exit 1; }

# happy path: pinned, signed, verified, backups outside the web root
fresh; "$here/publish-feed.sh"
want=$(printf 'hello\n' | sha256sum | cut -d' ' -f1)
[ "$(jq -r '.mods[0].sha256' "$T/root/mods.json")" = "$want" ] || { echo "FAIL: hash not filled"; exit 1; }
[ "$(jq -r '.mods[1].sha256' "$T/root/mods.json")" = abc ] || { echo "FAIL: existing pin changed"; exit 1; }
[ -f "$T/bk/mods.json" ] && [ ! -e "$T/root/mods.json.bak" ] || { echo "FAIL: backup missing or inside the web root"; exit 1; }
ls "$T/root" | grep -q '\.new$' && { echo "FAIL: temp file left"; exit 1; }
"$here/publish-feed.sh" --check >/dev/null || { echo "FAIL: --check should now pass"; exit 1; }

# failure paths leave the live files byte-identical (including the old signatures)
fresh; before=$(snap); rm "$T/key.pem"
"$here/publish-feed.sh" >/dev/null 2>&1 && { echo "FAIL: missing key should fail"; exit 1; }
[ "$before" = "$(snap)" ] || { echo "FAIL: missing key changed live files"; exit 1; }
: > "$T/key.pem"
fresh; before=$(snap)
FAIL_SIGN=1 "$here/publish-feed.sh" >/dev/null 2>&1 && { echo "FAIL: failing sign should fail"; exit 1; }
[ "$before" = "$(snap)" ] || { echo "FAIL: failing sign changed live files"; exit 1; }
fresh; before=$(snap)
AD_SIGN_URL=x SIGN_FEED=/nonexistent "$here/publish-feed.sh" >/dev/null 2>&1 && { echo "FAIL: missing tool should fail"; exit 1; }
[ "$before" = "$(snap)" ] || { echo "FAIL: missing tool changed live files"; exit 1; }
echo "publish-feed test ok"
