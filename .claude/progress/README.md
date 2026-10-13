# Progress entries

One file per entry, so two PRs never edit the same lines of PROGRESS.md.

- Name: `YYYY-MM-DD-short-slug.md`. First line: `# YYYY-MM-DD Title`. Then what changed, what is CONFIRMED / NOT verified, and the next step.
- Add a new file for new work. Edit your own file when you update it. Do not edit other threads' files except to correct a fact.
- Read: `python3 .claude/progress/now.py [N]` (newest first). Validate: `python3 .claude/progress/now.py --check`.
- PROGRESS.md keeps only Done / Broken / Next and a pointer here. Hook v2 counts changes in this folder as the PROGRESS update.
