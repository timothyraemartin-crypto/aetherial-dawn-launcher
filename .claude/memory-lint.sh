#!/usr/bin/env bash
# Lint .claude/memory/: prints "ERROR: ..." / "WARN: ..." lines. Exit 1 if any ERROR.
# Rules: INDEX.md <= 2048 B; topic <= 3072 B; frontmatter name/description/type/verified;
# every topic is listed in INDEX.md (topics in a subfolder, e.g. reviews/x.md, are linked from a hub topic instead) and every INDEX link exists; every `refs:` path exists;
# no secrets. Warns on topics not verified in 90+ days or more than 25 topics.
set -u
cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0
dir=.claude/memory
[ -d "$dir" ] || exit 0
err=0
e() { echo "ERROR: $*"; err=1; }
w() { echo "WARN: $*"; }
size() { wc -c < "$1" | tr -d ' '; }

[ -f "$dir/INDEX.md" ] || { e "$dir/INDEX.md is missing"; exit 1; }
[ "$(size $dir/INDEX.md)" -le 2048 ] || e "INDEX.md is $(size $dir/INDEX.md) B (cap 2048): shorten entries or merge topics"

n=0
for f in "$dir"/*.md "$dir"/*/*.md; do
  [ -f "$f" ] || continue
  b=$(basename "$f" .md); [ "$b" = INDEX ] && continue
  rel=${f#$dir/}
  n=$((n+1))
  [ "$(size "$f")" -le 3072 ] || e "$b.md is $(size "$f") B (cap 3072): split it or cut it down"
  fm=$(awk 'NR==1&&$0!="---"{exit} NR>1&&$0=="---"{exit} NR>1{print}' "$f")
  get() { printf '%s\n' "$fm" | sed -n "s/^$1: *//p" | head -n1; }
  [ "$(get name)" = "$b" ] || e "$b.md: frontmatter name must equal the file name"
  [ -n "$(get description)" ] || e "$b.md: missing description"
  case $(get type) in gotcha|decision|contract|howto|reference) ;; *) e "$b.md: type must be gotcha|decision|contract|howto|reference" ;; esac
  v=$(get verified)
  if printf '%s' "$v" | grep -Eq '^[0-9]{4}-[0-9]{2}-[0-9]{2}$'; then
    vs=$(date -d "$v" +%s 2>/dev/null) && [ $(( $(date +%s) - vs )) -gt 7776000 ] && w "$b.md last verified $v (90+ days): re-check it, then bump verified"
  else e "$b.md: verified must be YYYY-MM-DD"; fi
  for r in $(get refs | tr ',' ' '); do [ -e "$r" ] || e "$b.md: ref '$r' no longer exists - update or delete this memory"; done
  grep -Eq '(ghp_|github_pat_|sk-[A-Za-z0-9]{20}|BEGIN [A-Z ]*PRIVATE KEY|[Pp]assword *[:=]|[Tt]oken *[:=] *[A-Za-z0-9]{16})' "$f" && e "$b.md looks like it contains a secret - remove it"
  case $rel in
    */*) grep -qs "]($rel)" "$dir"/*.md || e "$rel is not linked from a hub topic in $dir/" ;;
    *) grep -q "]($b.md)" "$dir/INDEX.md" || e "$b.md is not listed in INDEX.md" ;;
  esac
done
for l in $(grep -o ']([a-z0-9-]*\.md)' "$dir/INDEX.md" | sed 's/^](//; s/)$//'); do
  [ -f "$dir/$l" ] || e "INDEX.md links to $l, which does not exist"
done
[ "$n" -le 25 ] || w "$n topics: merge or delete the stale ones"
exit $err
