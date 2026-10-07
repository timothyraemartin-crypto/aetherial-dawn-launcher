# Memory index (always loaded; hard cap 2 KB)
Durable facts a fresh session cannot get from the code. Read a topic only when its line applies.

Save: one fact per `<slug>.md` (<= 3 KB). Frontmatter: name (= slug), description, type (gotcha|decision|contract|howto|reference), verified (YYYY-MM-DD), refs (repo paths it depends on). Body: the fact, then **Why:** and **How to apply:**. Add one line below. Update instead of duplicating; delete when wrong or when the code now says it. No secrets, no task state (that is PROGRESS.md), nothing git or the code already shows. Check with `bash .claude/memory-lint.sh`.
Cross-repo facts are also in the project memory; repo-specific ones live here.

## Topics
- [live-mod-list](live-mod-list.md) - before answering "which mods" or editing the mod list / server load order
