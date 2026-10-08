# Handoff: <task>

- Task/session: <stable task ID and observed session ID>
- Delivery state: COMPLETE_WITHIN_SCOPE / NEEDS_GAME_CHECK / BLOCKED / IN_PROGRESS
- Scope: <what this outcome covers>
- Updated at: <timestamp>
- Working directory/repo/branch/commit: <observed identity; include dirty state>
- Objective and acceptance criteria: <observable outcome>
- Changes: <paths and behavior>
- Verification: <command receipts, runtime evidence, failures/skips and reasons>
- Prior attempts: <what failed, why, and retry conditions>
- Unresolved: <actual limitations and dependencies>
- Current ownership: <files/working tree owned by this task>
- Next action: <one executable continuation>
- Shared records: <synced location or pending-sync status>
- Release/deployment: <separate actual status, not implied by completion>

This durable summary complements the structured JSON handoff; it does not replace
machine-recorded checks or turn manual references into verified evidence.
