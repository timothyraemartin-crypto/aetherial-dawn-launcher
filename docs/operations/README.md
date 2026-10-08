# Durable operations records

These are templates, not completed evidence. Keep the project's existing shared
location where it already serves this purpose; link to it instead of duplicating
volatile facts here. Runtime logs remain under the ignored .claude/runtime folder.

Use unique IDs such as `DEC-20261008-<short-unique-suffix>` and
`INV-20261008-<short-unique-suffix>`. Do not use mutable line numbers as IDs.
Use ISO 8601 timestamps with timezone. Each record identifies repo/environment,
evidence status, source revision, and what would invalidate the conclusion.

Templates:
- DECISION_TEMPLATE.md — choices, reasons, consequences, supersession.
- INVESTIGATION_TEMPLATE.md — symptoms, hypotheses, failed attempts, root cause.
- HANDOFF_TEMPLATE.md — continuation that another thread can execute.
- GAME_CHECK_TEMPLATE.md — the minimum precise manual runtime observation.
- DEPLOYMENT_TEMPLATE.json — actual artifact/target identity; null means unknown.
- HISTORICAL_AUDIT_LEADS.md — prior observations to verify, never current truth.

Promote reusable, sanitized conclusions from runtime logs into durable records.
Do not copy tokens, credentials, raw player identifiers, or unrelated private data.
