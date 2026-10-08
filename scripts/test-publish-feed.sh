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
 {"id":"pinned","name":"P","url":"https://x.test/p.zip","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
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
[ "$(jq -r '.mods[1].sha256' "$T/root/mods.json")" = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ] || { echo "FAIL: existing pin changed"; exit 1; }
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

# Before the launcher release there is no sign-feed to download yet (the newest release predates it):
# --check must still report unpinned and non-https entries instead of dying on the download.
export AD_RELEASE_BASE=file:///nonexistent
fresh; before=$(snap)
out=$(SIGN_FEED= PATH="$T/nobin:$PATH" "$here/publish-feed.sh" --check 2>&1) && { echo "FAIL: --check without a tool should still flag the unpinned entry"; exit 1; }
echo "$out" | grep -q "Entries without sha256: local" || { echo "FAIL: unpinned entry not named without a tool: $out"; exit 1; }
[ "$before" = "$(snap)" ] || { echo "FAIL: --check changed files"; exit 1; }
fresh; jq '.mods[0].sha256="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" | .mods += [{"id":"plain","name":"H","url":"http://x.test/h.zip","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]' "$T/root/mods.json" > "$T/m" && mv "$T/m" "$T/root/mods.json"
out=$(SIGN_FEED= "$here/publish-feed.sh" --check 2>&1) && { echo "FAIL: --check should flag the non-https url"; exit 1; }
echo "$out" | grep -q "not https (the launcher drops them): plain" || { echo "FAIL: non-https entry not named: $out"; exit 1; }
fresh; jq '.mods[0].sha256="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"' "$T/root/mods.json" > "$T/m" && mv "$T/m" "$T/root/mods.json"
out=$(SIGN_FEED= "$here/publish-feed.sh" --check 2>&1) || { echo "FAIL: all pinned, --check should pass without a tool: $out"; exit 1; }
echo "$out" | grep -q "signatures not checked" || { echo "FAIL: skipped signature check not said: $out"; exit 1; }

# --pin-only: the step to run BEFORE the release. Needs no tool and no key, fills sha256 and nothing else.
fresh; rm -f "$T/root/mods.json.sig" "$T/root/client/manifest.json.sig"; : > "$T/root/untouched"
cp "$T/root/client/manifest.json" "$T/manifest.before"
SIGN_FEED= AD_FEED_KEY=/nonexistent "$here/publish-feed.sh" --pin-only >/dev/null || { echo "FAIL: --pin-only should work with no tool and no key"; exit 1; }
[ "$(jq -r '.mods[0].sha256' "$T/root/mods.json")" = "$want" ] || { echo "FAIL: --pin-only did not pin"; exit 1; }
[ ! -e "$T/root/mods.json.sig" ] && [ ! -e "$T/root/client/manifest.json.sig" ] || { echo "FAIL: --pin-only must not sign"; exit 1; }
cmp -s "$T/root/client/manifest.json" "$T/manifest.before" || { echo "FAIL: --pin-only touched the manifest"; exit 1; }
[ -f "$T/bk/mods.json" ] || { echo "FAIL: --pin-only kept no backup"; exit 1; }
# a list that already has a signature would be left with a stale one: refuse and change nothing
fresh; before=$(snap)
SIGN_FEED= "$here/publish-feed.sh" --pin-only >/dev/null 2>&1 && { echo "FAIL: --pin-only must refuse a signed list"; exit 1; }
[ "$before" = "$(snap)" ] || { echo "FAIL: refused --pin-only changed files"; exit 1; }
# A short or non-hex sha256 is not a pin: the launcher drops the entry, so --check flags it and the pin step replaces it.
fresh; jq '.mods[1].sha256="abc" | .mods[1].url="https://example.test/launcher/patches/a.ini" | .mods += [{"id":"nothex","name":"X","url":"https://example.test/launcher/patches/a.ini","sha256":"'"$(printf 'z%.0s' $(seq 64))"'"}]' "$T/root/mods.json" > "$T/m" && mv "$T/m" "$T/root/mods.json"
out=$(SIGN_FEED= "$here/publish-feed.sh" --check 2>&1) && { echo "FAIL: --check should flag a short sha256"; exit 1; }
echo "$out" | grep -q "Entries without sha256: local pinned nothex" || { echo "FAIL: short/non-hex pins not named: $out"; exit 1; }
rm -f "$T/root/mods.json.sig" "$T/root/client/manifest.json.sig"
SIGN_FEED= "$here/publish-feed.sh" --pin-only >/dev/null || { echo "FAIL: --pin-only should repair short pins"; exit 1; }
jq -e '[.mods[] | select(.url != null and .nexus == null) | .sha256 | test("^[0-9a-fA-F]{64}$")] | all' "$T/root/mods.json" >/dev/null || { echo "FAIL: --pin-only left an invalid sha256"; exit 1; }
# A nexus entry with a `file` and a short sha256 is dropped by the launcher, so --check must name it.
fresh; jq '.mods[0].sha256="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" | .mods[2].file="Data/x.ini" | .mods[2].sha256="abc123"' "$T/root/mods.json" > "$T/m" && mv "$T/m" "$T/root/mods.json"
out=$(SIGN_FEED= "$here/publish-feed.sh" --check 2>&1) && { echo "FAIL: --check should flag the nexus entry with a short sha256"; exit 1; }
echo "$out" | grep -q "no full sha256 (the launcher drops them): nx" || { echo "FAIL: nexus file entry not named: $out"; exit 1; }
jq '.mods[2].sha256="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"' "$T/root/mods.json" > "$T/m" && mv "$T/m" "$T/root/mods.json"
out=$(SIGN_FEED= "$here/publish-feed.sh" --check 2>&1) || { echo "FAIL: a full sha256 on the file entry should pass: $out"; exit 1; }
echo "publish-feed test ok"
