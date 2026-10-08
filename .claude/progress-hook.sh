#!/usr/bin/env bash
# Claude Code hook: keeps PROGRESS.md the source of truth and checks the build.
#   start -> SessionStart: show PROGRESS.md, remember the starting commit
#   stop  -> Stop: block if code changed without a PROGRESS.md update, if the build check fails,
#            or if .claude/memory/ changed and fails memory-lint.sh
set -u
cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0
mode=${1:-}
input=$(cat 2>/dev/null || true)
sid=$(printf '%s' "$input" | sed -n 's/.*"session_id" *: *"\([^"]*\)".*/\1/p' | head -n1)
mark="${TMPDIR:-/tmp}/ad-progress-${sid:-none}"

# Fast build check for this repo; print problems and return non-zero on failure.
build_check() {
  # Rust core must compile; JS/JSON in ui/ must parse.
  command -v cargo >/dev/null && changed_files | grep -q -e "\.rs$" -e "Cargo\.\(toml\|lock\)$" && { cargo check -p launcher-core --tests --quiet 2>&1 | tail -n 20; [ "${PIPESTATUS[0]}" -eq 0 ] || return 1; }
  for f in $(changed_files | grep "\.js$"); do [ -f "$f" ] && { node --check "$f" || return 1; }; done
  return 0
}

changed_files() {
  base=$(cat "$mark" 2>/dev/null || true)
  [ -n "$base" ] || base=$(git merge-base HEAD origin/HEAD 2>/dev/null || git rev-parse HEAD 2>/dev/null)
  { git diff --name-only "$base" 2>/dev/null; git ls-files --others --exclude-standard 2>/dev/null; } | sort -u
}

case $mode in
start)
  git rev-parse HEAD > "$mark" 2>/dev/null || true
  echo "RULE (Timothy, 2026-10-07): before ANY work, read the project memory, PROGRESS.md and .claude/memory/INDEX.md and name them in your first status line; before fixing a bug read .claude/memory/root-causes.md."; echo "PROGRESS.md is this repo's source of truth. Read it before starting; update it EVERY time you start, fix or build something (Now / Done / Broken). Also update .claude/memory: when you fix a finding a topic lists, edit or delete that entry; add new durable facts."
  echo "----- PROGRESS.md -----"
  if [ -f PROGRESS.md ]; then head -n 40 PROGRESS.md; else echo "(missing - create PROGRESS.md before finishing)"; fi
  if [ -f .claude/memory/INDEX.md ]; then
    echo "----- MEMORY (.claude/memory/INDEX.md; Read a topic file only when its line applies) -----"
    cat .claude/memory/INDEX.md
    bash .claude/memory-lint.sh 2>&1 | head -n 10
  fi
  ;;
stop)
  # Already continued once because of this hook: let the session end.
  printf '%s' "$input" | grep -q '"stop_hook_active" *: *true' && exit 0
  files=$(changed_files)
  code=$(printf '%s\n' "$files" | grep -v -e '^$' -e '^PROGRESS\.md$' -e '^\.claude/' -e '\.md$')
  msg=""
  if printf '%s\n' "$files" | grep -q '^\.claude/memory/'; then
    mout=$(bash .claude/memory-lint.sh 2>&1) || msg="Memory check failed (.claude/memory/INDEX.md has the rules): $(printf '%s' "$mout" | grep '^ERROR' | head -n 8 | tr '\n' ' ')"
  fi
  if [ -n "$code" ]; then
    printf '%s\n' "$files" | grep -qx 'PROGRESS.md' || msg="${msg}Code changed but PROGRESS.md was not updated. Add what you fixed/built (and anything still broken or next) to PROGRESS.md, and update any .claude/memory topic whose finding you fixed. "
    out=$(build_check 2>&1) || msg="${msg}Build check failed - fix it, then re-run: ${out:0:1500}"
  fi
  if [ -n "$msg" ]; then
    esc=$(printf '%s' "$msg" | sed 's/\\/\\\\/g; s/"/\\"/g' | awk '{printf "%s\\n", $0}')
    printf '{"decision":"block","reason":"%s"}\n' "$esc"
  fi
  ;;
esac
exit 0
