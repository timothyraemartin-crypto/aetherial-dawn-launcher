# Aetherial Dawn startup

Protocol version: 2.0.0. Read .claude/ad-system/OPERATING_PROTOCOL.md on adoption or version change; revisit relevant sections for the task.

- Resolve the actual task, workspace, repo, branch/commit, and existing changes. Preserve unrelated work.
- Read PROGRESS.md, .claude/memory/INDEX.md, relevant decisions, and prior failed attempts. Validate volatile facts against their primary source.
- Treat the old audit as historical observations, not current repo/deployment truth.
- Define observable acceptance criteria, implement a coherent change, and run relevant real checks. Prefer the simplest existing local route. Do not introduce runners or infrastructure without a task need and authorization.
- The SessionStart hook supplies context and a session ID; it does not prove understanding. Use that ID with the check/handoff commands documented in the package README.
- Read-only work does not require a build. Changed source requires an accurate progress update and structured handoff. Explicitly record blocked or pending game verification.
- Keep implementation, checks, merge, build, deployment, and runtime verification separate. Never fabricate access, commands, results, versions, or approval.
- Use actual deployment preflight before deployment. A Stop hook is not a release barrier.
- Continue authorized reversible work without repeated confirmation. Ask only when a material unresolved decision or permission blocks the next action.
- Keep chat concise and records detailed. Report result, evidence, remaining limits, and next action.

The protocol follows platform controls and current user instructions. It grants no additional access or permissions.
