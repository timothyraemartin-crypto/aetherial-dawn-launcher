# Per-change quality checklist (Standing Goal section 6)

Copy this block into the PR description or the thread's handoff. Tick a box **only** with evidence beside it
(test result, `file:line`, log line, screenshot, CI run, or a reasoned "N/A: reason"). "Not tested" stays unticked.
Label evidence CONFIRMED / REPORTED / INFERRED / UNKNOWN. In-game effects stay UNKNOWN or INFERRED until someone has watched them, then GAME-VERIFIED.

```
Change: <title, PR link>   Register IDs: <IU-... / LA-... / DI-...>   Date: <YYYY-MM-DD>
Stage reached (one only): Planned | Implemented | Offline-tested | Staging-verified | Live-verified | Monitored | Blocked (<why>)

- [ ] Scope and acceptance criteria defined.            Evidence:
- [ ] Actual integration points identified.             Evidence: (modules, config files, launcher/bot contracts, deploy path, live server)
- [ ] Implementation complete for the stated scope.     Evidence: (diff re-read against the ask)
- [ ] Permissions and trusted identity verified.        Evidence: (who may call it; server-side check, not button hiding)
- [ ] Input validation and failure behavior verified.   Evidence:
- [ ] Data integrity and metadata preservation verified where applicable.   Evidence: (item signatures, counts, no dup/loss)
- [ ] Concurrency, replay, interruption, and recovery tested where applicable.   Evidence: (restart mid-operation, double fire, reconnect)
- [ ] Relevant automated tests passed.                  Evidence: (command, counts, CI run URL)
- [ ] Existing related behavior regression-tested.      Evidence:
- [ ] UI/input/focus behavior tested where applicable.  Evidence: (browser test / screenshot / focus hand-off)
- [ ] Performance impact assessed at an appropriate scale.   Evidence: (timers, per-tick cost, message rate)
- [ ] Native staging verification completed where required.  Evidence: (staging port 7790 once T6 staging exists; else N/A with reason)
- [ ] Deployment and rollback instructions prepared.    Evidence: (backup path, kill switch, old binary, revert PR)
- [ ] Documentation and feature register updated.       Evidence: (register row, PROGRESS.md entry)
- [ ] Remaining limitations explicitly recorded.        Evidence:

Four separate words (never fold into "done"):
implemented: yes/no   checked: <what>   merged: yes/no (sha)   deployed: yes/no (time)   game-verified: yes/no (who, when)
Existing failures vs new regressions: <list each red test and whether it was red on main before this change>
Verdict: COMPLETE_WITHIN_SCOPE | NEEDS_GAME_CHECK | BLOCKED | IN_PROGRESS
```

Three checks (project rule, 2026-10-13): 1 re-read change against ask; 2 verify the artifact itself; 3 verify the effect.
Do not skip, weaken or delete a test or safety check to get green. A mocked crash test or sequential writes are not proof of atomicity.
