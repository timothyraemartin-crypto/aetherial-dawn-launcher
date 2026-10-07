#!/usr/bin/env bash
# Offline test of scripts/publish-feed.sh with a stand-in sign-feed.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
mkdir -p "$T/root/patches"; echo hello > "$T/root/patches/a.ini"; : > "$T/key.pem"
cat > "$T/root/mods.json" <<JSON
{"mods":[
 {"id":"local","name":"L","url":"https://example.test/launcher/patches/a.ini"},
 {"id":"pinned","name":"P","url":"https://x.test/p.zip","sha256":"abc"},
 {"id":"nx","name":"N","nexus":{"mod":1}}]}
JSON
printf '#!/bin/sh\necho "$@" >> "%s/calls"\n' "$T" > "$T/sign-feed"; chmod +x "$T/sign-feed"
export AD_WEB_ROOT="$T/root" AD_FEED_KEY="$T/key.pem" AD_BASE_URL=https://example.test/launcher SIGN_FEED="$T/sign-feed"

"$here/publish-feed.sh" --check && { echo "FAIL: --check should flag the unpinned entry"; exit 1; }
[ ! -e "$T/root/mods.json.bak" ] || { echo "FAIL: --check changed files"; exit 1; }
"$here/publish-feed.sh"
want=$(printf 'hello\n' | sha256sum | cut -d' ' -f1)
[ "$(jq -r '.mods[0].sha256' "$T/root/mods.json")" = "$want" ] || { echo "FAIL: hash not filled"; exit 1; }
[ "$(jq -r '.mods[1].sha256' "$T/root/mods.json")" = abc ] || { echo "FAIL: existing pin changed"; exit 1; }
grep -q "^sign $T/key.pem $T/root" "$T/calls" && grep -q "^verify $T/root" "$T/calls" || { echo "FAIL: sign/verify not called"; exit 1; }
"$here/publish-feed.sh" --check >/dev/null || { echo "FAIL: --check should now pass"; exit 1; }
echo "publish-feed test ok"
