# 2026-10-14 Custom Skills Framework stage 1: build workflow and menu file (nothing ships)

Rebased the closed PR 140 onto main: manual workflow `build-csf.yml` (builds Exit-9B/CustomSkills at pinned commit 8be7055f on a GitHub Windows machine, uploads CustomSkills.dll + sha256), `native-mods/csf/` with the MIT notice, README and the generated `SKILLS.json` (18 skills, 192 perks; regenerated from current ingame-ui main + PR 350, byte-identical to the file from PR 140). `bundled.rs`, rules and the feed are untouched.
Decision (Timothy 2026-10-14 00:10Z): stage 1 = stock framework to see whether the Tab menu opens on 1.6.1170; fork (live + hide non-picks) only if it works. Ingame-ui side: draft PR 350.
HELD (coordinator): bundling the DLL, guard allowlist entry in launcher-rules.json, feed publish, any release, until launcher 0.1.154 black-screen fix is released and Timothy has played once without a black screen.
NOT run: the build workflow has never run; NOT compiled, merged, released or game-verified. UNKNOWN: pinned commit builds, 1.6.1170 support.
