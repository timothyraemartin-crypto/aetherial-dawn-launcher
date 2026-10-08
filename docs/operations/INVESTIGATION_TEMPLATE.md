# Investigation: <symptom>

- ID: <unique stable ID>
- Evidence state: REPORTED / INFERRED / CONFIRMED
- Status: OPEN / FIXED_LOCALLY / NEEDS_RUNTIME_CHECK / VERIFIED / SUPERSEDED
- Repo/version/environment: <actual observed identity>
- Reported at / last verified: <timestamps>
- Impact: <affected behavior and scope>
- Reproduction: <setup, steps, expected, actual>
- Baseline evidence: <sanitized logs/screenshots/metrics and references>

## Hypotheses and attempts

| Time | Hypothesis | Action | Observation | Conclusion | Retry condition |
|---|---|---|---|---|---|
| <time> | <mechanism> | <exact change/check> | <actual result> | <what it rules in/out> | <new evidence needed> |

## Cause and remedy

- Confirmed cause: <mechanism and discriminating evidence, or UNKNOWN>
- Remaining hypothesis: <what is still inferred>
- Fix: <change and why it addresses the cause>
- Regression evidence: <check, covered source, result, environment>
- Deployment/runtime evidence: <separate from code checks>
- Remaining limits: <unverified conditions>
- Prevention: <proportionate guard or test>
- Invalidated when: <version/environment/contract change>
- Related decisions/fixes: <repo-qualified references and stable IDs>
