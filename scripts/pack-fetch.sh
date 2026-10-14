#!/usr/bin/env bash
# Downloads, for one Nexus collection revision, only the files a pre-flight can judge (SKSE plugin DLLs,
# plugins, the Address Library file) from each mod archive into OUTDIR/<modId>-<fileId>/, then
# `pack-preflight scan OUTDIR` reads them. Textures, meshes and sound are never extracted.
# Needs NEXUS_API_KEY (never echoed) and 7z. Read-only toward Nexus and the repo.
#   scripts/pack-fetch.sh REVISION OUTDIR      MAX_ARCHIVE_MB=300 skips bigger archives and says so
# First CI run (PR 192, rev 7): 19 archives extracted, 7 'download failed' (spaces in the link, fixed here; RaceMenu, FNIS, XPMSE among them), 3 over 300 MB or no DLL.
set -euo pipefail
cd "$(dirname "$0")/.."
RULES=server/launcher-rules.json
SLUG=$(jq -r .collection.slug "$RULES"); GAME=$(jq -r .collection.game "$RULES"); REV=$1; OUT=$2
MAX_MB=${MAX_ARCHIVE_MB:-300}
G='https://api.nexusmods.com/v2/graphql'
mkdir -p "$OUT"; W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
LINK=$(curl -fsS -m 60 -X POST $G -H 'content-type: application/json' -d "$(jq -n --arg s "$SLUG" --arg g "$GAME" --argjson r "$REV" '{query:"{ collectionRevision(slug:\"\($s)\", revision:\($r), domainName:\"\($g)\", viewAdultContent:true){ downloadLink } }"}')" | jq -r .data.collectionRevision.downloadLink)
URL=$(curl -fsS -m 60 -H "apikey: $NEXUS_API_KEY" -H 'accept: application/json' "https://api.nexusmods.com$LINK" | jq -r '.download_links[0].URI // .download_links[0].uri // empty')
curl -fsS -m 600 -o "$W/c.7z" "$URL"; 7z e -y -o"$W/x" "$W/c.7z" collection.json >/dev/null
cp "$W/x/collection.json" "$OUT/collection.json"
WANT='\.(dll|esp|esm|esl)$|versionlib-.*\.bin$'
: > "$OUT/fetch-report.txt"
for pair in $(jq -r '.mods[] | select(.source.modId != null) | "\(.source.modId):\(.source.fileId)"' "$W/x/collection.json"); do
  m=${pair%:*}; f=${pair#*:}; D="$OUT/$m-$f"
  META=$(curl -fsS -m 60 -H "apikey: $NEXUS_API_KEY" "https://api.nexusmods.com/v1/games/$GAME/mods/$m/files/$f.json" || echo '{}')
  KB=$(jq -r '.size_kb // 0' <<<"$META"); P=$(jq -r '.content_preview_link // empty' <<<"$META")
  if [ -n "$P" ]; then
    TREE=$(curl -fsS -m 60 "${P// /%20}" | jq -r '[.. | objects | select(.type=="file") | .path] | .[]' 2>/dev/null || true)
    if [ -n "$TREE" ] && ! grep -qiE "$WANT" <<<"$TREE"; then echo "mod $m file $f: no DLL/plugin in the archive (content preview), skipped" >>"$OUT/fetch-report.txt"; continue; fi
  fi
  if [ "$KB" -gt $((MAX_MB * 1024)) ]; then echo "mod $m file $f: $((KB / 1024)) MB > $MAX_MB MB, NOT CHECKED" >>"$OUT/fetch-report.txt"; continue; fi
  U=$(curl -sS -m 60 -H "apikey: $NEXUS_API_KEY" "https://api.nexusmods.com/v1/games/$GAME/mods/$m/files/$f/download_link.json" | jq -r '.[0].URI // empty' || true)
  if [ -z "$U" ]; then echo "mod $m file $f: no download link, NOT CHECKED" >>"$OUT/fetch-report.txt"; continue; fi
  curl -fsS -m 900 -o "$W/a.bin" "${U// /%20}" || { echo "mod $m file $f: download failed, NOT CHECKED" >>"$OUT/fetch-report.txt"; continue; }
  mkdir -p "$D"; 7z x -y -o"$D" "$W/a.bin" '-ir!*.dll' '-ir!*.esp' '-ir!*.esm' '-ir!*.esl' '-ir!versionlib-*.bin' >/dev/null || true
  echo "mod $m file $f: extracted $(find "$D" -type f | wc -l) file(s)" >>"$OUT/fetch-report.txt"; rm -f "$W/a.bin"
done
cat "$OUT/fetch-report.txt"
