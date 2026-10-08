---
name: root-causes
description: Past bugs as symptom -> cause -> fix, so you do not retry what already failed
type: gotcha
verified: 2026-10-08
---
One line per cause: symptom -> root cause -> fix (PR). Add a line whenever you find a root cause; edit it if the fix changes; delete it if the code now makes it impossible. Never fix a symptom twice: check here first.

- Windows job red on PRs -> GitHub-hosted jobs refused (failed account payments) -> self-hosted Linux runner; Windows build only on release tag/manual (#75).
- Mod list or manifest edited, clients unchanged -> signed feed not republished -> run "Publish launcher feed" after every edit (signed-feeds).
- Vortex check blocked sign-in when Vortex missing -> check was fail-closed -> fail-open (Timothy's decision).
- Auto-fixes in launcher don't reach players -> they only ship with the next release tag -> cut a release after merging.
- Windows UAC prompt from a remote session is cancelled -> needs admin terminal -> give Timothy a command only when he is home.
- A fixed toast scrolled away inside Settings -> an ancestor with backdrop-filter (the scrolling .sheet) becomes the containing block for position: fixed -> keep fixed elements outside any element with backdrop-filter/transform/filter.
- Play button jumped up when the F3 hint, What's new or a Play-anyway prompt appeared -> the dock is bottom-anchored, so rows added below Play push it up -> put growing rows above Play and reserve a fixed slot for the hint and status below it.
