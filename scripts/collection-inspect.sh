#!/usr/bin/env bash
# Prints what a Nexus collection revision carries, for writing detection files and the plugin order:
# the collection.json keys, its plugin list, and for each mod the archive's file tree (Nexus content preview).
# Needs NEXUS_API_KEY (never echoed). Read-only; changes nothing.  scripts/collection-inspect.sh REVISION
set -euo pipefail
cd "$(dirname "$0")/.."
RULES=server/launcher-rules.json
SLUG=$(jq -r .collection.slug "$RULES"); GAME=$(jq -r .collection.game "$RULES"); REV=$1
G='https://api.nexusmods.com/v2/graphql'
LINK=$(curl -fsS -m 60 -X POST $G -H 'content-type: application/json' -d "$(jq -n --arg s "$SLUG" --arg g "$GAME" --argjson r "$REV" '{query:"{ collectionRevision(slug:\"\($s)\", revision:\($r), domainName:\"\($g)\", viewAdultContent:true){ downloadLink } }"}')" | jq -r .data.collectionRevision.downloadLink)
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
URL=$(curl -fsS -m 60 -H "apikey: $NEXUS_API_KEY" -H 'accept: application/json' "https://api.nexusmods.com$LINK" | jq -r '.download_links[0].URI // .download_links[0].uri // empty')
curl -fsS -m 600 -o "$W/c.7z" "$URL"; 7z e -y -o"$W/x" "$W/c.7z" collection.json >/dev/null
echo "== collection.json top-level keys"; jq -c 'keys' "$W/x/collection.json"
echo "== plugins"; jq -c '.plugins' "$W/x/collection.json"
echo "== pluginRules"; jq -c '.pluginRules' "$W/x/collection.json"
echo "== modRules"; jq -c '.modRules' "$W/x/collection.json" | head -c 3000; echo
echo "== choices of the new mods"; jq -c '.mods[] | select(.choices != null) | {name, choices}' "$W/x/collection.json" | head -c 6000; echo
echo "== mods (name, source ids)"; jq -c '.mods[] | {name, folderName, source: (.source | {type, modId, fileId, fileSize}), choices: (.choices | type)}' "$W/x/collection.json"
echo "== file trees"
for pair in $(jq -r '.mods[] | select(.source.modId != null) | "\(.source.modId):\(.source.fileId)"' "$W/x/collection.json"); do
  m=${pair%:*}; f=${pair#*:}
  P=$(curl -fsS -m 60 -H "apikey: $NEXUS_API_KEY" "https://api.nexusmods.com/v1/games/$GAME/mods/$m/files/$f.json" | jq -r '.content_preview_link // empty')
  echo "-- mod $m file $f"
  case $m in 94244|101101|59652|44666) [ -n "$P" ] && curl -fsS -m 60 "${P// /%20}" | jq -r '[.. | objects | select(.type=="file") | .path] | .[]' | sed -E 's#^[^/]+/00 Data/[^/]+/##' | grep -iE '^(meshes|textures|sound|interface|scripts)/' | cut -d/ -f1-3 | tr 'A-Z' 'a-z' | sort -u | head -150;; esac
  [ -n "$P" ] && curl -fsS -m 60 "${P// /%20}" | jq -r '[.. | objects | select(.type=="file") | .path] | .[]' | grep -iE '\.(esp|esm|esl|dll|ini|json|toml|bin)$|^[^/]*/?[^/]*$' | grep -viE '/(textures|meshes|sound|scripts|interface)/' | head -60 || echo "(no preview)"
done
