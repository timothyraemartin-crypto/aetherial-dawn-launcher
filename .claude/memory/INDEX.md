# Memory index (always loaded; hard cap 2 KB)
Durable facts a fresh session cannot get from the code. Read a topic only when its line applies.

Save: one fact per `<slug>.md` (<= 3 KB). Frontmatter: name (= slug), description, type (gotcha|decision|contract|howto|reference), verified (YYYY-MM-DD), refs (repo paths it depends on). Body: the fact, then **Why:** and **How to apply:**. Add one line below. Update instead of duplicating; delete when wrong or when the code now says it. No secrets, no task state (that is PROGRESS.md), nothing git or the code already shows. Check with `bash .claude/memory-lint.sh`.
Questions for Timothy go through the project chat, not your thread (send_message to the channel session).
Before ANY work: read project memory, PROGRESS.md and this index; name them in your first status line. Before fixing a bug read root-causes.md; add a line when you find a cause.

## Topics
- [root-causes](root-causes.md) - before fixing any bug: past symptom -> cause -> fix
- [live-mod-list](live-mod-list.md) - before answering "which mods" or editing the mod list / server load order
- [repo-links](repo-links.md) - before changing auth, bans/kicks, or anything crossing repos
- [launcher-parts](launcher-parts.md) - before editing the launcher or when asked what a part does
- [kept-files](kept-files.md) - before deleting a file that looks unused
- [vortex-extension-review](vortex-extension-review.md) - before changing vortex-extension/ or the Vortex bridge
- [quality-reviews](quality-reviews.md), [-2](quality-reviews-2.md), [-3](quality-reviews-3.md), [-4](quality-reviews-4.md) - open findings: sign-in/UI (1), file sync (2), mod list/Vortex (3), Tauri shell (4)
- [signed-feeds](signed-feeds.md) - before editing mods.json/manifest on the server or the feed signing code
- [launcher-improvements](launcher-improvements.md) - before planning new launcher features
