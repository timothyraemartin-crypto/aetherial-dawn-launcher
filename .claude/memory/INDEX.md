# Memory index (always loaded; hard cap 2 KB)
Durable facts a fresh session cannot get from the code. Read a topic only when its line applies.

Save: one fact per `<slug>.md` (<= 3 KB). Frontmatter: name (= slug), description, type (gotcha|decision|contract|howto|reference), verified (YYYY-MM-DD), refs (repo paths it depends on). Body: the fact, then **Why:** and **How to apply:**. Add one line below. Update instead of duplicating; delete when wrong or when the code now says it. No secrets, no task state (that is PROGRESS.md), nothing git or the code already shows. Check with `bash .claude/memory-lint.sh`.
Cross-repo facts are also in the project memory; repo-specific ones live here.

## Topics
- [live-mod-list](live-mod-list.md) - before answering "which mods" or editing the mod list / server load order
- [repo-links](repo-links.md) - before changing auth, bans/kicks, or anything crossing repos
- [launcher-parts](launcher-parts.md) - before editing the launcher or when asked what a part does
- [kept-files](kept-files.md) - before deleting a file that looks unused
- [vortex-extension-review](vortex-extension-review.md) - before changing vortex-extension/ or the Vortex bridge
- [quality-reviews](quality-reviews.md) - before touching sign-in, health checks, plugin order, load order or ui/ (open findings)
