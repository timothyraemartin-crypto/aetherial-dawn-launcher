---
name: root-causes
description: Past bugs as symptom -> cause -> fix, so you do not retry what already failed
type: gotcha
verified: 2026-10-08
---
One line per cause: symptom -> root cause -> fix (PR). Add a line whenever you find a root cause; edit it if the fix changes; delete it if the code now makes it impossible. Never fix a symptom twice: check here first.

- Windows job red on PRs -> GitHub-hosted jobs were refused for billing (#75 moved it to the PC) -> since 0.1.134 (#115) the Windows build is back on windows-latest, release tags and manual runs only; PRs use the Linux checks.
- Mod list or manifest edited, clients unchanged -> signed feed not republished -> run "Publish launcher feed" after every edit (signed-feeds).
- Vortex check blocked sign-in when Vortex missing -> check was fail-closed -> fail-open (Timothy's decision).
- Auto-fixes in launcher don't reach players -> they only ship with the next release tag -> cut a release after merging.
- Windows UAC prompt from a remote session is cancelled -> needs admin terminal -> give Timothy a command only when he is home.
- A fixed toast scrolled away inside Settings -> an ancestor with backdrop-filter (the scrolling .sheet) becomes the containing block for position: fixed -> keep fixed elements outside any element with backdrop-filter/transform/filter.
- Play button jumped up when the F3 hint, What's new or a Play-anyway prompt appeared -> the dock is bottom-anchored, so rows added below Play push it up -> put growing rows above Play and reserve a fixed slot for the hint and status below it.
- Play still moved in error states -> the status line sat below Play and grew downward with long text -> every row that changes height sits above Play; only the fixed hint slot is below.
- Player opens a book and the game breaks (reported 2026-10-11, not reproduced; freeze/crash/stuck unknown) -> NoPause 1.0.5 had Book on its unpause allow-list, so the book menu ran with the world live -> bundled AetherialDawn-NoPause.ini sets Book=0 (DLL unchanged); needs an in-game book test.
- Player's Skyrim starts but the server never sees a connect (Lambcut 2026-10-11, launcher fine up to the SKSE loader, no crash report) -> INFERRED Windows Firewall Block rule for SkyrimSE.exe or UDP 7777 blocked -> launcher repairs the firewall rules, probes the UDP port and flags "ran but never connected" (netcheck.rs; kill switches launcher.firewallFix/udpProbe/connectWatch); needs a game check. 2026-10-12 correction: for Lambcut the firewall was NOT the cause (UDP 7777 answered, firewall WARN stayed, he joined). His game reached the main menu with SKSE, Skyrim Platform and the client script loaded but Skyrim Platform's in-game browser never started (no browser backend lines, no Temp\\Skyrim Platform folder, helper not running), so no login box. It cleared after manual steps on his PC on plain 0.1.137; Timothy: "something to do with his files". Cause INFERRED (a file/AV-level problem on his PC); not game-verified which step. Also CONFIRMED in his 0.1.138 report: Skyrim rewrote plugins.txt after start (148 CC files first) although the launcher set and verified it just before launch; SkyMP still connected, so only the launch-time order matters. Preventive checks: browsercheck.rs (ini backend, temp folder, browser file list, Defender/security product lines in the report), connect ladder (ladder.rs), 15 s watch.
- Players could see sample skills (blacksmithing 2, archery 1, fishing 3) in the skill tree -> the bundled AetherialSkillTree.test.json is the fallback when the server state (GMST sTestFile2) is not found -> stop shipping it and remove our unchanged copy (branch claude/qa-launcher-gkb1yf).
