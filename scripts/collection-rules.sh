#!/usr/bin/env bash
# Rebuilds the `collection` object of server/launcher-rules.json for a Nexus
# collection revision (public GraphQL API, no key). Detection files and the
# plugin order are not in the API, so they are kept from the current file by
# modId; a mod new to the revision stops the script until its files are added.
#   scripts/collection-rules.sh [REVISION]     (default: the current file's revision + 1)
# Then publish with: sudo scripts/publish-feed.sh --rules server/launcher-rules.json
set -euo pipefail
cd "$(dirname "$0")/.."
RULES=server/launcher-rules.json
SLUG=$(jq -r .collection.slug "$RULES"); GAME=$(jq -r .collection.game "$RULES")
REV=${1:-$(( $(jq -r .collection.revision "$RULES") + 1 ))}
Q=$(jq -n --arg s "$SLUG" --arg g "$GAME" --argjson r "$REV" '{query:"{ collectionRevision(slug:\"\($s)\", revision:\($r), domainName:\"\($g)\", viewAdultContent:true){ revisionNumber revisionStatus modFiles{ optional version file{ fileId version mod{ modId name } } } } }"}')
OUT=$(curl -fsS -m 60 -X POST https://api.nexusmods.com/v2/graphql -H 'content-type: application/json' -d "$Q")
echo "$OUT" | jq -e '.data.collectionRevision.revisionStatus == "published"' >/dev/null || { echo "revision $REV of $SLUG not found or not published"; exit 1; }
NEW=$(echo "$OUT" | jq --slurpfile old "$RULES" '
  ($old[0].collection.mods | map({key: (.modId|tostring), value: .files}) | from_entries) as $files
  | [.data.collectionRevision.modFiles[] | select(.optional | not) | {name: .file.mod.name, modId: .file.mod.modId, fileId: .file.fileId, version: .file.version} | . + {files: ($files[(.modId|tostring)] // [])}]')
MISSING=$(echo "$NEW" | jq -r '.[] | select(.files | length == 0) | "\(.name) (\(.modId))"')
[ -z "$MISSING" ] || { echo "add detection files (game-relative) for these mods in $RULES first, then rerun:"; echo "$MISSING"; exit 1; }
jq --argjson mods "$NEW" --argjson rev "$REV" '.collection.mods = $mods | .collection.revision = $rev | .rev = (now | strftime("%Y-%m-%d.1"))' "$RULES" > "$RULES.tmp" && mv "$RULES.tmp" "$RULES"
jq '.collection' "$RULES" > core/data/collection-$SLUG-r$REV.json
echo "revision $REV written: $(echo "$NEW" | jq length) mods. Review the diff; the built-in copy in core/data is for the next launcher release."
