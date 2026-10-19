# 30-day pilot scorecard template

Create one private copy per pilot. Use a pilot code in any shared aggregate
summary; keep customer names, repository paths, source and raw CI logs inside
the customer's environment. Obtain the customer's permission before sharing
even aggregate numbers.

## Scope and baseline, completed before install

| Field | Record |
|---|---|
| Pilot code and internal owner | |
| Product owner and security reviewer | |
| Two or more repositories and their types | |
| CI platform and runner OS/architecture | |
| Current gate/script and reviewer workflow | |
| Planned release gates (count and definitions) | |
| Baseline reviewer minutes per release | |
| Baseline gate p50/p95 latency, with sample size | |
| Baseline false pass/fail incidents, adjudicated | |
| Source, log, credential, network and residency constraints | |
| Candidate binary SHA, run URL and attestation verification | |
| Approved spec revision and command trust decision | |
| Rollback owner and retained previous gate | |

## Weekly record

Copy this row for weeks 1–4. Compare the same type of release or CI event
against the baseline where possible; record any changed workload.

| Week | Repo type / OS | Runs | Planned gates | Gates represented | Adjudicated false passes | Adjudicated false failures | Gate p50 / p95 seconds | Verifiable records / runs | Reviewer minutes / release | Unplanned maintenance hours | Exceptions and owner | Notes |
|---|---|---:|---:|---:|---:|---:|---|---:|---:|---:|---|---|
| 1 | | | | | | | | | | | | |
| 2 | | | | | | | | | | | | |
| 3 | | | | | | | | | | | | |
| 4 | | | | | | | | | | | | |

Record install-to-first-pass in elapsed minutes, starting when the approved
candidate is available to the team and ending at the first passing spec run.
For gate coverage, divide represented planned gates by the agreed planned
gate count; mark exclusions explicitly. For evidence completeness, divide
independently verifiable gate records by all runs. A command output or a local
checksum alone is not an independently verifiable record. If a metric has no
data, write “not measured” instead of zero.

## Day-30 review

Interview the gate operator and release approver separately. Record their
observed review burden, false result incidents, unmet checks, procurement
blockers, and whether they would continue weekly use. Apply the exit gates in
[the India roadmap](../../INDIA_MARKET_READINESS.md) using measured evidence.
Record the decision as **continue**, **fix and repeat**, or **stop/reposition**,
with owner and date. Do not count a prepared template as a live pilot.
