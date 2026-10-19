# India market readiness and enterprise roadmap for `deliver`

_Assessment date: 14 October 2026. Product baseline: local working tree of `deliver` v0.3.0. This is a market and product assessment, not a claim of regulatory certification or measured customer demand._

## Executive decision

**Proceed with a focused India discovery and pilot program; do not position the current binary as enterprise ready.** The best initial buyers are engineering platform and release governance teams in India based global capability centres (GCCs) and large IT services firms. They already run heterogeneous repositories and CI systems, and can trial a local CLI without sending source code to a new service. Regulated financial institutions are a second wave after security, evidence and procurement controls are proven. Public sector procurement is a later option, not the launch segment.

`deliver` should be sold as a **deterministic, repository local deliverable and release gate** that complements tests, policy engines and CI. Its differentiation must be verifiable evidence about _which spec, code revision and checks passed_, plus simple adoption across many repositories. A generic “AI compliance” or “India compliant” claim would outrun the product and the evidence.

The India opportunity is credible but not sized: GitHub reports **21.9 million India based developers in 2025**; Nasscom and Zinnov report **2,117 Indian GCCs, 3,728 units and 2.36 million GCC staff** in FY2026. These are ecosystem indicators, **not** buyers, seats or a revenue forecast. Neither source measures demand for `deliver`. [GitHub Octoverse 2025](https://github.blog/news-insights/octoverse/octoverse-a-new-developer-joins-github-every-second-as-ai-leads-typescript-to-1/) · [Nasscom–Zinnov GCC landscape FY2026](https://zinnov.com/centers-of-excellence/zinnov-nasscom-india-gcc-landscape-2026-report/)

## Evidence and assessment method

Product claims below were checked against this checkout's `README.md`, `Cargo.toml`, `src/`, tests and GitHub Actions workflows on the assessment date. The working tree already contains substantial uncommitted changes from other work; those are part of the observed baseline, not released capability. A feature in an unreferenced source file is counted as **not shipped**. Market facts come from the linked primary publishers. The judgments, priorities and targets in this document are recommendations and hypotheses, not survey results.

| Dimension | Readiness | Evidence and gap |
|---|---|---|
| Core product | **Moderate** | TOML/JSON specs; file, directory, command and Git checks; spec composition and JSON Schema; stable text/JSON/SARIF/JUnit output; parallel checks. The product solves a concrete gate problem. |
| Adoption | **Low** | CLI and pre-commit support exist. No published India pilots, implementation case studies, packaged policy templates for target workflows, or validated buyer interviews. |
| Enterprise security | **Low** | A spec can launch arbitrary commands with inherited environment and an arbitrary working directory. There is no trust boundary, execution policy, output size limit, subprocess-tree termination guarantee, or documented threat model. Treat specs from forks, agents and external repositories as executable code. |
| Evidence and audit | **Low** | Reports summarize outcomes but do not bind results cryptographically to spec, commit, tool build and input hashes. There is no verifier, signature or retention/export contract. Existing SARIF results lack source locations. |
| Distribution and operations | **Low–moderate** | CI builds on Linux, macOS and Windows; no release signing or provenance workflow is evident. Installation, version pinning, upgrade, rollback, support and offline delivery are not enterprise ready. |
| India fit | **Conditional** | Local execution can fit source control and restricted-network environments, but this needs verified zero network behavior, local evidence handling and customer-specific deployment review. No India-specific legal certification is warranted. |

Current documentation has drift: `ROADMAP.md` still says checks run sequentially in one section although `--jobs` is implemented; `BENCHMARKS.md` describes v0.1.2 and sequential execution although `Cargo.toml` is v0.3.0. `src/cache.rs` and `src/incremental.rs` exist as untracked files but are not connected to the CLI or `lib.rs`; incremental validation and caching must remain unshipped in external claims. This audit is a point-in-time reading, not a full security test.

## India demand and buyer thesis

| Segment | Problem to validate | Likely buyer and pilot user | Priority |
|---|---|---|---|
| GCC platform teams | Standardize release gates across globally owned repositories, languages and CI providers; show remote stakeholders what passed. | India engineering platform lead; release engineering or developer experience team. | **1** |
| IT services / product engineering firms | Produce consistent, client-specific acceptance evidence across many delivery teams without building one-off shell scripts. | Delivery assurance lead; DevSecOps practice. | **1** |
| BFSI and fintech engineering | Demonstrate controlled change and audit evidence for internal governance and vendor due diligence. | CISO/IT governance with platform engineering as implementer. | **2**, after security review |
| Public sector and critical infrastructure | Offline deployment, procurement evidence and organization policy controls. | Procurement plus IT security. | **3**, only if pilots show pull |

India's developer and GCC scale supports a **distribution thesis**, not a top-down TAM calculation. Do not multiply developer counts by a seat price. The initial economic unit should be a governed repository or team, with pricing tested through pilots against time saved in release review and audit preparation. The CLI should remain usable without a paid service; enterprise value would come from support, signed policy bundles, fleet rollout and evidence verification only when customers request them.

**Competitive boundary.** Conftest tests structured configuration with Rego and supports shared policy bundles; Checkov scans infrastructure and code for security misconfigurations with a large built-in rule catalogue. GitHub Actions provides CI execution and artifact attestations. `deliver` should integrate with these rather than replace their policy languages, scanners or build provenance. Its proposed niche is a small cross-repository acceptance spec with portable evidence for file, command, directory and Git gates. This niche is a hypothesis until users prefer it to their existing CI scripts. [Conftest overview](https://www.conftest.dev/) · [Conftest policy sharing](https://www.conftest.dev/sharing/) · [Checkov documentation](https://github.com/bridgecrewio/checkov/blob/main/docs/1.Welcome/What%20is%20Checkov.md) · [GitHub artifact attestations](https://docs.github.com/en/actions/concepts/security/artifact-attestations)

## India regulatory and procurement implications

1. **Personal data.** India's final Digital Personal Data Protection Rules, 2025 were notified in November 2025 with staged commencement. A purely local CLI need not receive customer personal data; hosted telemetry, support uploads, central evidence storage or user accounts would change the data flow and require a legal and security review. Document the data inventory, purpose, retention and deletion before any such service is designed. Avoid claiming blanket data localisation from the DPDP Rules. [MeitY final rules and timeline](https://www.meity.gov.in/documents/act-and-policies/digital-personal-data-protection-rules-2025-gDOxUjMtQWa?pageTitle=Digital-Personal-Data-Protection-Rules-2025%C3%AF%C2%BF%C2%BC) · [Gazette text and commencement clauses](https://www.meity.gov.in/static/uploads/2025/11/53450e6e5dc0bfa85ebd78686cadad39.pdf)
2. **Cyber incidents and logs.** CERT-In's 2022 directions include six-hour reporting for specified incidents and 180-day ICT log retention in India for covered entities. Whether and how this applies to a particular customer or a future hosted `deliver` service needs counsel and customer assessment; a local validator cannot make the customer compliant by itself. Enterprise incident playbooks should make logs exportable and preserve timestamps, rather than assert that `deliver` satisfies a retention obligation. [CERT-In directions](https://www.cert-in.org.in/PDF/CERT-In_Directions_70B_28.04.2022.pdf) · [CERT-In FAQ](https://www.cert-in.org.in/PDF/FAQs_on_CyberSecurityDirections_May2022.pdf)
3. **Financial sector.** RBI's 2023 IT governance directions and 2023 IT outsourcing directions affect regulated entities and their vendor oversight. For BFSI buyers, supply security documentation, audit access terms, incident processes and continuity information; map controls with each buyer. A binary installed inside a customer's CI is not automatically an outsourced IT service. [RBI IT governance directions](https://systemhealth.rbi.org.in/Scripts/BS_ViewMasDirections.aspx_id%3D12562%283%29.html) · [RBI IT outsourcing directions](https://systemhealth.rbi.org.in/Scripts/BS_ViewMasDirections.aspx_id%3D12486.html)

## Product and commercial roadmap

Dates are elapsed time from an approved kickoff, not committed release dates. Phase gates require evidence; failing a gate means fix the issue or narrow the market before advancing. One small core engineering team plus part-time security and customer discovery support is assumed. Estimates should be reforecast after the first two pilots.

| Phase | Time | Deliverables and owner | Exit gate |
|---|---|---|---|
| **0. Validate demand and baseline** | Weeks 0–4 | Product lead: 15–20 structured buyer/user interviews across GCC and IT services teams; record current gate, decision maker, procurement blocker, willingness to trial. Engineering: reconcile README/roadmap/benchmark claims; publish supported-platform matrix; capture baseline install-to-first-pass, runtime and false-failure rates. Security: threat model specs, paths, processes, outputs and secrets. | At least **5 qualified design partners** with named owners and **3** agreeing to a scoped pilot; reproducible baseline and ranked threat list. Otherwise reassess positioning. |
| **1. Safe, repeatable pilot** | Weeks 4–10 | Engineering/security: explicit trusted-spec model; fail closed on unknown spec keys; path containment and symlink tests; bounded stdout/stderr; process-tree timeout/cleanup; secret redaction; no telemetry by default; deterministic report IDs and schema version; Linux/macOS/Windows test matrix. Product: 2–3 templates (generic release, Rust, mixed repository), offline install guide, migration guide from shell CI gates. | Security review closes critical findings; CI passes on supported platforms; **3 live pilots**, each with ≥2 repositories, no source data egress, documented rollback and reproducible pass/fail results. |
| **2. Verifiable evidence and distribution** | Weeks 10–18 | Engineering: signed release binaries, checksums, SBOM, build provenance and independent verification instructions; spec/commit/input digests in evidence bundle; explicit cached vs executed status; stable machine-readable report contract. Security: provenance verification and key rotation runbook. Docs: source-linked claims and versioned migration policy. | Independent party verifies a release and an evidence bundle offline; each pilot can retrieve a complete gate record; ≥95% of pilot gate runs produce valid evidence; no known critical vulnerabilities. |
| **3. Fleet adoption and paid design** | Weeks 18–28 | Product/engineering: centrally versioned, pinned policy bundle distribution with local override rules and provenance; GitHub/GitLab/Jenkins examples; policy rollouts and exceptions with owner, reason and expiry; performance work only after measured bottlenecks. Commercial: India support hours, SLA draft, security questionnaire, pilot conversion offer. | **2 referenceable design partners** (permissioned), ≥10 active governed repositories across pilots, one repeatable deployment path, quantified review-time reduction; at least **2 paid pilot or procurement intents** before committing to a hosted control plane. |
| **4. Regulated/enterprise expansion** | After phase 3 | Build only from buyer pull: private evidence store or customer-managed export, retention controls, SSO/RBAC for any central service, audit API, disaster recovery, accessibility and procurement pack. Legal/security: buyer-specific RBI/DPDP/CERT-In mapping and contract terms. | One regulated buyer completes security/procurement review and production pilot, with control ownership explicit; support burden and unit economics remain viable. |

### Engineering priority within the phases

**P0 — before external pilots:** model specs as code execution; constrain base and check paths; reject silent typos in specs; set resource and output limits; verify cancellation kills descendants; ensure reports cannot leak secret-bearing stdout/stderr. Add adversarial tests for malicious or malformed specs, symlink traversal, command hangs and parallelism. Use existing `--strict` semantics as the blocking gate; a weighted completion score must never override failure.

**P1 — evidence:** version the report schema; include exact tool version/build digest, spec hash, repository revision and dirty state, check inputs and timestamps; record command exit status and whether a result was executed or replayed. Sign and verify evidence through a documented trust root. A content hash alone proves integrity, not who produced the evidence. Follow existing artifact attestation mechanisms where practical instead of inventing a standalone PKI. [GitHub artifact attestations](https://docs.github.com/en/actions/concepts/security/artifact-attestations)

**P2 — fleet operations:** conditional checks, pinned shared specs and exception workflow can reduce duplicated gate definitions. Integrate proven scanners via command checks first; add a plugin protocol only after repeated unmet needs, because it expands the execution attack surface. Defer streaming NDJSON, broad auto-fix and a dashboard until pilots show a specific workflow bottleneck. Connect and benchmark the unfinished cache/incremental modules only after correctness, invalidation and evidence semantics are defined.

## Pilot design and measurements

Run each pilot for 30 days with a current CI script as comparator. Collect only opt-in aggregate metrics; keep source and raw artifacts in the customer's environment. Measure: time from install to first passing spec; percentage of planned release gates represented; false pass/false fail count (manually adjudicated); median and p95 gate latency; percentage of runs with verifiable evidence; reviewer minutes per release; unplanned maintenance hours; security or data handling exceptions. Segment by platform and repo type. Interview the engineer using the gate and the person approving releases separately.

**Go/no-go after phase 2:** continue if at least three pilots use the gate weekly, all critical security issues are closed, ≥95% of runs yield independently verifiable records and at least two buyers report a measured reduction in review effort. If pilots only use it as a file-existence wrapper, simplify packaging and revisit differentiation before adding a control plane. These are management thresholds, not market statistics.

## Risks and immediate decisions

| Risk | Mitigation / decision |
|---|---|
| An agent or pull request changes `deliver.toml` to execute arbitrary commands | Require trusted review/pinning of specs; isolate untrusted CI jobs; document inherited environment and runner privileges; make safe execution a phase-1 blocker. |
| Audit evidence is mistaken for compliance | Phrase it as evidence of configured checks on a defined revision; require buyer-specific control mapping and legal review. |
| CI scripts are “good enough” | Test against actual scripts and report the saved reviewer time and maintenance burden; stop if no measurable benefit. |
| Large enterprise feature list stalls a small product | Release in gated increments and let pilot evidence decide which integrations to build. |
| Source and telemetry handling blocks procurement | Keep the default local and offline; publish a data-flow diagram and support data-handling process before collecting any customer artifacts. |

**Next four weeks:** name a product owner and security reviewer; complete the threat model and documentation audit; recruit five qualified India design partners through direct outreach; select three pilots only after their current gate and success metric are documented. No revenue forecast or compliance claim should be published until these inputs exist.

## Research limitations

The Ami CLI classified this checkout as alpha, but its dependency-derived search phrases returned **zero prospects** in this run; it provides no India demand validation. No customer interviews, paid contracts, India-specific conversion rates or independently verified installed base were available. Government rules and buyer obligations should be rechecked at contract time. All numerical pilot gates and schedule estimates above are planning assumptions.
