# Jcode: Surpass OMP in the Remaining Weak Areas

**Implementation plan · September 6, 2026**

**Status:** Proposed implementation program. No implementation or benchmark victory is claimed by this document.

**Suggested repository location:** `docs/plans/omp-overtake/REMAINING_GAPS_IMPLEMENTATION_PLAN.md`

| Baseline | Pinned revision |
|---|---|
| `LLMpsycho/jcode`, `master` | `25b9f90f4be17d90289754681155cc21d562f592` |
| `LLMpsycho/oh-my-pi`, `main` | `8553cd681ad27014d05cf5b9708ef3322975a409` |

This is a **delta plan**, not a replacement runtime or a restart of the existing OMP-overtake program. Keep `MASTER_PLAN.md` as the index and link this plan from it. Preserve the completed advisor work and the existing editing, LSP, DAP, SDK, swarm, and evaluation foundations. Recheck branch heads before implementation; the revisions above are the source-review baseline, not moving references. [S1][S2][S3]

---

## Contents

1. [Objective and corrections to the earlier comparison](#1-objective-and-corrections-to-the-earlier-comparison)
2. [Success criteria and score policy](#2-success-criteria-and-score-policy)
3. [Architecture and non-negotiable constraints](#3-architecture-and-non-negotiable-constraints)
4. [Phases and dependency order](#4-phases-and-dependency-order)
5. [S0 — Establish the actual gaps and lock the evaluation](#5-s0--establish-the-actual-gaps-and-lock-the-evaluation)
6. [S1 — Shared execution, permission, and lifecycle contracts](#6-s1--shared-execution-permission-and-lifecycle-contracts)
7. [S2 — Extend existing declarative agent profiles](#7-s2--extend-existing-declarative-agent-profiles)
8. [S3 — Reliable composition and verified multi-agent results](#8-s3--reliable-composition-and-verified-multi-agent-results)
9. [S4 — Advisor lifecycle, context, and recovery hardening](#9-s4--advisor-lifecycle-context-and-recovery-hardening)
10. [S5 — Unified configuration and agent controls](#10-s5--unified-configuration-and-agent-controls)
11. [S6 — A bounded extension platform](#11-s6--a-bounded-extension-platform)
12. [S7 — Authoring, compatibility, and reference packages](#12-s7--authoring-compatibility-and-reference-packages)
13. [S8 — Prove the result and release safely](#13-s8--prove-the-result-and-release-safely)
14. [Pull-request sequence](#14-pull-request-sequence)
15. [Cross-cutting regression matrix](#15-cross-cutting-regression-matrix)
16. [Implementation handoff and completion checklist](#16-implementation-handoff-and-completion-checklist)
17. [Sources](#17-sources)

---

## 1. Objective and corrections to the earlier comparison

### 1.1 Objective

Make jcode better than the pinned OMP fork in the four areas that previously received lower scores:

- Advisor/watchdog behavior.
- Multi-agent specialization and composition.
- Tools and extensibility.
- Configuration and user experience.

Preserve exact authenticated model routing, private provider conversations, bounded supervision, session durability, permission enforcement, and runtime efficiency. More features are not an improvement when they introduce silent identity changes, unsolicited continuation, configuration ambiguity, or unsafe parallel writes.

### 1.2 Important correction: several proposed “missing” features already exist

The preceding comparison was too categorical about declarative agents and `/agents`, and understated the advisor implementation. The current source shows the following:

| Capability | Current evidence | Correct next step |
|---|---|---|
| Declarative custom profiles | `agent_profile.rs` loads Markdown/YAML profiles from the user agent directory and `.jcode/agents`, overlays project definitions, and supports descriptions, tools, effort, and role instructions. | Extend this loader and contract; do not create a competing registry or file format. |
| `/agents` control surface | `inline_interactive/agent_models.rs` already displays main, swarm, advisor, review, judge, memory, ambient, and named profiles. | Unify scope, live status, editing, and remote behavior rather than introducing `/agents` again. |
| Profile model behavior | The profile parser has no profile-specific model/route field; its catalog and UI explicitly describe retaining the configured swarm model. | Add explicit per-profile model binding with backward-compatible migration and authenticated resolution. |
| Investigative advisor | The runtime has an `advise` tool, investigative calls, bounded/coalesced updates, and retained exchanges. | Improve context budgeting, diagnostics, recovery, and measured intervention quality. |
| Advisor continuity | `history.rs` retains complete exchanges and the original objective, with byte/exchange limits. | Add model-aware budgeting and explicit loss/recovery semantics, not “first-time memory.” |
| Advisor delivery | Current delivery code distinguishes terminal findings, cancellation, actual delivery, and interruption immunity. | Verify and harden the state machine under races and restarts. |
| Competitive evaluation | `scripts/competitive_eval` already has isolated runs, adapter fingerprints, locked schemas/hashes, timeouts, and result handling. | Extend it; do not build another evaluation framework. |
| CI guardrails | Existing CI checks formatting, compilation, Clippy, budgets, dependency boundaries, and SDK parity. | Add focused coverage to existing jobs; do not create a workflow for every phase. |

These are source-inspection findings, not claims that all relevant runtime tests passed in this planning session. [S3][S4][S5][S6][S7][S8][S9][S19][S20]

### 1.3 The narrower, defensible gap

OMP exposes richer task-agent contracts, including structured-output validation and isolation options, and configurable advisor tooling and delivery behavior. Its advisor runtime also contains context-maintenance and failure-recovery machinery. These are useful behavioral references—not proof of higher task success. [S13][S14][S15][S16][S17]

The implementation target is therefore:

> Extend jcode's existing profiles, supervision, and controls into a consistent, measurable platform—without sacrificing their existing boundaries.

---

## 2. Success criteria and score policy

### 2.1 Historical scores are prioritization inputs, not measurements

| Area | Earlier jcode score | Earlier OMP score | Product ambition, not a forecast |
|---|---:|---:|---|
| Advisor | 9.2 | 9.7 | Leadership in useful, timely, low-noise supervision. |
| Multi-agent | 9.2 | 9.5 | Leadership in specialization, composition, and verified outcomes. |
| Tools/extensibility | 9.1 | 9.7 | Leadership on the selected extension-authoring and integration workflows. |
| Configuration/UX | 9.1 | 9.4 | Leadership in configuration success, transparency, and recovery. |

The earlier `92.7` versus `93.6` totals were subjective code-review assessments. They were not a benchmark, and the corrections above mean they should not be treated as a calibrated baseline. Do not promise a future `98/100` by completing checkboxes.

S0 must freeze a new rubric and apply it to **both** pinned implementations. Do not compare a new measured score directly with the old subjective scores.

### 2.2 Category-specific evidence

| Category | Primary outcome | Supporting measures | Mandatory floor |
|---|---|---|---|
| Advisor | Seeded defects corrected in the final artifact without another user prompt. | Clean-task interruption rate; precision/recall; useful-note latency; extra tokens/cost; retained-objective accuracy. | No continuation after explicit stop; no unauthorized tools or evidence recipients. |
| Multi-agent | Deterministically verified completion of composed repository tasks. | Schema-valid results; duplicate work; merge conflicts; coordinator repair; cost; recoverable worker failure. | No silent overwrite, unverified success, or cross-session ownership leak. |
| Extensibility | Completion of predeclared extension-authoring/integration tasks without changing jcode core. | Host crashes; compatibility; setup steps; cancellation; permission prompts; unload cleanup. | No unauthorized activation or capability expansion. |
| Configuration/UX | Successful completion of model/profile/advisor configuration tasks. | Errors, recovery steps, interaction count, completion time, and agreement between displayed and actual settings. | No wrong-scope writes or misreported active model/account. |

**Initial engineering targets:** all deterministic contracts pass; zero critical violations in the adversarial suite; no more than 5% of clean advisor tasks produce an interrupt; no material regression against the current jcode baseline on correctness. These are proposed acceptance thresholds, not observed results. S0 must finalize and lock their operational definitions before feature implementation.

### 2.3 What counts as surpassing OMP

Predeclare one primary comparison per category and report uncertainty. Use paired, task-clustered analysis; repeated attempts on one task are not independent tasks. Control multiple comparisons across the four primary claims.

A category qualifies as a win through either:

1. **Quality leadership:** a statistically supported improvement in its primary outcome, with all safety and regression floors met.
2. **Efficiency leadership at equivalent quality:** predeclared non-inferiority on quality, plus a supported material cost/latency improvement and no safety loss. Label this narrower claim explicitly.

Proposed margins for S0 registration: 2 percentage points for quality non-inferiority; 10% cost reduction or 20% latency reduction for efficiency leadership. These margins must be justified against the task mix and frozen before implementation results are seen.

Run matched-model, matched-route, matched-budget trials first. A separate “best supported configuration” track may use each product's native strengths under equal total budgets. Never mix those tracks into one unexplained score.

If the data cannot distinguish the competitors, report **inconclusive**. A richer interface or a small test-count advantage is not evidence of better coding outcomes. Extension workflow leadership also does not establish a larger third-party ecosystem.

---

## 3. Architecture and non-negotiable constraints

### 3.1 Reuse existing ownership boundaries

```text
TUI / CLI / SDK / ACP
          |
          v
Existing server control and session APIs
          |
          +-- Profile resolution + immutable execution snapshot
          +-- Existing swarm/task execution and result acceptance
          +-- Existing advisor manager and delivery queue
          +-- Existing tool registry + common permission evaluator
          +-- Extension contributions through those same contracts
```

Stable DTOs remain in existing low-dependency contract crates where appropriate. Parsing, filesystem access, provider resolution, and lifecycle management remain in runtime/foundation modules. TUI code renders and requests changes; it must not become the authority for remote execution policy. Avoid a crate-per-feature redesign. [S1]

### 3.2 Required invariants

**Model identity.** Resolve every role to an exact authenticated route. Preserve model, provider/runtime, account reference, supported effort, and provider-session separation. Never embed credentials in profiles, snapshots, logs, or extension manifests. Never use process-wide account activation to satisfy an isolated role. Existing route-pinned provider forks remain the foundation. [S10][S11]

**No silent fallback.** Retry transient transport failures within the selected route. Switching model, account, or provider requires a separately configured and approved recovery policy. Automatic context promotion is a model change too; it cannot bypass this rule.

**Permission intersection.** Effective capability is the intersection of platform/session policy, role ceiling, selected profile, invocation narrowing, extension grant, and current revocations. Missing and empty tool lists must have distinct documented meanings. Delegation cannot increase permission.

**Advisor boundary.** Keep advisors read-only investigators by default and preserve existing stricter ceilings. No shell, writes, arbitrary MCP calls, hidden primary reasoning, or delegated execution simply to match OMP's feature breadth. Broader investigation must be explicitly safe at the operation level. [S3][S12]

**Policy hooks.** Current investigative access may be unavailable when a pre-tool hook is configured. Do not bypass that policy to improve an evaluation result. Reuse a non-shell evaluator only after equivalence is tested; otherwise preserve the restriction and explain it. [S12]

**Cancellation and authority.** Explicit user stop wins over queued notes, retries, auto-review, and worker completion. Asynchronous advice can only influence future safe boundaries; it cannot retroactively prevent an action already executed. Pre-effect blocking belongs only to an explicitly configured guardian/policy gate.

**Durability.** Use generation fencing and idempotent publication. Do not claim exactly-once external effects across a crash. An interrupted shell, provider call, or extension action can have an uncertain outcome; preserve that status and do not blindly replay it.

**Filesystem trust.** Current canonical-path checks are not a hostile-filesystem sandbox. Treat workspace confinement and adversarial symlink/TOCTOU safety separately. Harden rooted reads where feasible; otherwise explicitly retain a trusted-workspace limitation instead of claiming sandbox security. [S3][S12]

**Configuration scope.** Preserve legacy user/project precedence. Add explicit scope and provenance rather than silently changing it. A project file cannot automatically authorize a new evidence recipient, executable extension, or broader capability.

**Lean delivery.** Work from owned branches/worktrees; preserve existing quality ratchets and unique CI coverage. Do not copy OMP code or prompts into jcode. Do not publish, merge, replace the shared daemon, or change global configuration as an incidental implementation step. [S1][S2]

### 3.3 Proposed shared contracts

These names describe proposed contracts; they do not assert that matching types already exist. S0/S1 should reuse equivalent current types.

| Contract | Required contents |
|---|---|
| `ResolvedAgentSpec` | Stable profile ID/version/hash, source/provenance, role, resolved route/effort, effective tool-operation grants, spawn policy, output contract, effective budgets. |
| `AgentExecutionSnapshot` | Owner/session/invocation IDs, generation, profile/config revision, resolved spec, original and current task requirements, workspace revision, budget reservation ID. |
| `AgentResult` | Success/failure/cancelled/unknown status, output validation status, structured data, artifact references, verifier evidence, actual route and usage attribution. |
| `AdvisorFinding` | Stable concern identity, advisor attribution, severity, evidence references/revision, proposed action, verification state, delivery state and disposition. |
| `AgentControlState` | Configured versus resolved values, source scope, live health, generation/config revision, pending changes, durable/failed write status. |

Snapshots are immutable for an invocation. Permission revocation and cancellation may narrow them immediately; ordinary config edits take effect at the next documented boundary.

### 3.4 Resolution and override rules

Resolve values in this order, then validate the complete result before admission:

| Setting | Resolution rule |
|---|---|
| Profile definition | Preserve the existing project-over-user name resolution; record the selected file and hash. Never merge two role prompts silently. |
| Model selector | Authorized per-invocation choice, then explicit named-session override, then v1 profile binding, then the existing role's saved/default selector. An unavailable explicit selector fails; it does not fall through to a lower priority. |
| Legacy model behavior | Unversioned swarm profiles retain the current swarm selector. Preserve specialized memory/ambient defaults; do not replace every role default with the primary model. |
| Explicit inheritance | Resolve against the documented parent/role source when the invocation starts and freeze that exact route for the invocation. “Inherit” is an explicit selector, not an error-recovery fallback. |
| Effort | Explicit invocation effort, then profile effort, then the existing role/model default; validate against the selected model. Invalid explicit effort fails visibly. |
| Tools/spawning | Intersect grants and all role/session restrictions; a higher-priority config cannot override a denial. A profile's self-declared kind cannot raise its role ceiling. |
| Budgets | Respect every applicable per-run cap and remaining root reservation. Child limits cannot mint new root capacity. |
| Output contract | An authorized caller may select a contract explicitly; otherwise use the profile contract. It cannot relax a parent-required schema or strictness. Reject incompatible contracts before spawning. |

An “apply now” control must explicitly cancel/fence the previous generation before starting one with changed execution semantics. Merely saving a file cannot mutate a running agent's model, role, or authority. State locks must not be held across provider, extension, or tool awaits.

---

## 4. Phases and dependency order

| Phase | Outcome | Primary categories | Entry dependency |
|---|---|---|---|
| S0 | Verified gap inventory and locked evaluation contract. | All | None |
| S1 | Shared execution, permission, and lifecycle contracts. | All; protects existing strengths | S0 |
| S2 | Richer backward-compatible declarative profiles. | Multi-agent, UX | S1 |
| S3 | Typed results and reliable composed execution. | Multi-agent | S2 |
| S4 | Advisor hardening and measured supervision quality. | Advisor | S1 |
| S5 | Consistent agent controls across clients and scopes. | UX | S2; advisor-specific integration after S4 |
| S6 | Bounded extension contributions and execution host. | Extensibility | S1, S2 |
| S7 | Authoring tools, compatibility importer, reference packages. | Extensibility, multi-agent, UX | S3, S6 |
| S8 | Final comparative campaign, migration/soak gates, controlled release. | All | S3, S4, S5, S7 |

```text
S0 -> S1 -> S2 -> S3 ----------------------> S8
       |      |       \                    ^
       |      +-> S6 --> S7 ---------------+
       +-> S4 --------> S5 ----------------+
              S2 -----> S5
```

S4 can proceed alongside S2/S3 once shared contracts are stable. Avoid parallel edits to the same server/protocol modules without an agreed ownership split. Phase numbers are dependency groups, not invitations to submit one enormous PR per phase.

---

## 5. S0 — Establish the actual gaps and lock the evaluation

**Goal:** Prevent duplicate implementation and prevent the scoring system from moving after results are known.

### Tasks

- [ ] Re-fetch `origin/master`; record the exact new base and changes since the pinned review. Read root and touched-directory `AGENTS.md` guidance.
- [ ] Map every requirement here to current source, tests, and runtime evidence. Classify it as `verified`, `implemented-unverified`, `partial`, `missing`, or `intentionally-excluded`.
- [ ] Trace profiles from discovery through `swarm spawn`/`assign_task`, child creation, execution, completion, restart, and UI. Parser support alone is not end-to-end support.
- [ ] Census existing task leases, budget accounting, result schemas, control APIs, and extension hooks before adding any equivalent service.
- [ ] Reconcile the existing roadmap and historical completion records. Keep old audit results as snapshots; do not present them as final-head results.
- [ ] Extend the existing evaluation manifests/adapters for the four target categories. Freeze fixture/verifier hashes, route settings, permissions, budgets, metric definitions, and competitor SHAs before implementation.
- [ ] Check that the OMP noninteractive adapter invokes the pinned fork correctly and preserves genuine failure/cancellation semantics. Unsupported behavior must remain visible, not silently skipped.
- [ ] Separate development fixtures from final holdouts. Keep verifiers outside agent-writable workspaces and keep holdouts unavailable to the implementation agent.
- [ ] Record the current jcode regression floor and baseline failure list without weakening required checks.

### Files

**Extend existing:** `docs/plans/omp-overtake/{MASTER_PLAN,SCORECARD,FAIRNESS,BASELINE_REPORT}.md`; `scripts/competitive_eval/`; existing adapter/schema/tests directories; `competitive-eval/` result storage.

**Proposed new:** `docs/plans/omp-overtake/REMAINING_GAPS_MATRIX.md` and a versioned remaining-gaps campaign manifest inside the existing harness. Add only if no equivalent document exists.

### Evaluation design

Use a pilot solely to estimate variance and required sample size. Predeclare minimum detectable effects and target statistical power; three repetitions per task are a minimum for a live campaign, not sufficient evidence by themselves. Randomize paired run order, isolate homes/memory, and capture model/provider drift. Keep clean tasks alongside defect tasks so a noisy advisor cannot win by interrupting everything.

Match permitted tools and information exposure in the controlled track. For example, do not give OMP primary reasoning or mutating advisor tools while claiming a like-for-like comparison with a read-only jcode advisor.

### Exit gate

The gap matrix and immutable campaign contract exist; existing implementations are not scheduled for replacement; baseline deficiencies are explicit. Feature PRs cannot edit their acceptance thresholds or hidden verifiers.

---

## 6. S1 — Shared execution, permission, and lifecycle contracts

**Goal:** Give new profile, advisor, and extension features one consistent execution contract without replacing the existing daemon or swarm.

### Tasks

- [ ] Resolve a `ResolvedAgentSpec` once at admission and carry an immutable `AgentExecutionSnapshot` through spawn, fork, transfer, queueing, and restart.
- [ ] Adapt existing role configuration and advisor roster entries into that contract. Preserve specialized advisor/review/judge execution rather than collapsing all roles into one generic loop.
- [ ] Centralize operation-level permission evaluation around the existing registry. Apply identical decisions to normal, batch, nested, delegated, and provider-hosted tool paths.
- [ ] Treat tool aliases and namespaced extension tools consistently. Unknown or unsupported permission names fail visibly; they never expand to a default tool set.
- [ ] Revalidate current revocation immediately before tool execution. Keep profile/tool discovery separate from permission to execute.
- [ ] Add owner/invocation/generation/config-revision fencing wherever missing. Reject old provider completions and stale worker results after cancellation, history rewrite, or replacement.
- [ ] Reuse current journaling/persistence for accepted control changes and pending deliveries. Where needed, add an outbox entry atomically with the state transition so a crash cannot silently lose a committed note.
- [ ] Give delivery entries idempotency keys. Mark accepted, queued, delivered, suppressed, acknowledged, or abandoned without claiming exactly-once external execution.
- [ ] Enforce root-level admission budgets across all children/advisors, not only per-agent limits. Reserve capacity before dispatch; reconcile actual usage after completion.
- [ ] Treat missing price/usage information as unknown. Enforce known token/step/concurrency ceilings instead of displaying a fictitious hard dollar cap.

### Files

**Existing integration points:** `crates/jcode-config-types/src/`; `crates/jcode-base/src/{provider/agent_roles.rs,session/}`; `crates/jcode-app-core/src/{server/,agent/,tool/}`; `crates/jcode-protocol/src/`; `crates/jcode-harness-api/src/`; `crates/jcode-harness-api-server/src/`.

**Proposed modules:** focused execution-snapshot and policy adapters under the existing base/app-core structure. Select their exact locations after the S0 dependency census; do not introduce a new central “common” crate.

### Required tests

Route/effort independence; parent unchanged by child configuration; two workspaces on one daemon; fork/transfer preservation; stale completion rejection; permission revocation during an await; nested-tool bypass; provider-hosted autonomous-tool rejection; budget races; crash before/after durable delivery admission.

### Exit gate and rollback

All consumers use the same resolved semantics for the paths being changed. No new permission path is controlled only by prompts or UI. Roll back by disabling new schema consumers while retaining readable additive persistence fields; do not erase journals to make rollback work.

---

## 7. S2 — Extend existing declarative agent profiles

**Goal:** Reach OMP's useful expressiveness through jcode's existing Markdown profiles, then improve provenance, validation, and route correctness.

### 7.1 Schema evolution

Retain the existing user agent directory and `.jcode/agents`. Preserve existing unversioned files and their execution semantics. Add a versioned profile contract with:

| Field family | Required semantics |
|---|---|
| Identity | Name, description, schema version, optional tags and role/kind. |
| Model binding | Inherit, existing role reference, or exact authenticated route; mutually exclusive selectors. |
| Effort | Validated against the resolved model, not merely against a static string list. |
| Tools | Existing aliases supported; requested tools/operations intersected with role and parent policy. |
| Spawn policy | Allowed child profile IDs, optional depth cap, concurrency, and cumulative root budgets. |
| Results | Optional local structured-output schema with strict validation mode. |
| Execution | Explicit foreground/background behavior and optional isolated-worktree requirement. |
| Provenance | Source file, content hash, scope, effective overrides, and trust decision. |

No profile can grant new credentials or execute its own setup shell command during discovery.

### 7.2 Proposed example—not currently supported syntax

```yaml
---
schema-version: 1
name: security-review
description: Review authorization and tenant-isolation changes.
kind: reviewer
model:
  role: review
effort: high
tools: [read, agentgrep]
spawns: []
output:
  schema: ./schemas/security-review-result.json
  mode: strict
budgets:
  max-model-steps: 12
  max-tool-calls: 24
---
Inspect the changed trust boundaries and cite concrete evidence.
Do not modify files. Distinguish verified findings from hypotheses.
```

`model.role` resolves through existing role settings to an exact private route. An explicit-route alternative must use the same authenticated catalog contract as `/agents`; a profile must not invent endpoints.

### 7.3 Tasks

- [ ] Extend `AgentProfile` and the parser rather than adding a second manifest registry.
- [ ] Add a legacy adapter and schema-version-aware validation. Unknown v1 semantic fields are errors. Legacy unknown fields produce actionable diagnostics without silently changing their old behavior.
- [ ] Specifically detect imported OMP `model`, `thinkingLevel`, `spawns`, `output`, and `blocking` metadata. Map supported semantics or report them as unsupported; never claim successful import while discarding them.
- [ ] Do not suddenly activate previously ignored model/tool fields in an old file. Require a previewed migration before newly effective settings can change cost, recipients, or permissions.
- [ ] Preserve existing project-over-user resolution. Show the winning source and shadowed definitions. Do not silently add ancestor-directory inheritance in this phase.
- [ ] Quarantine an invalid profile and preserve unrelated valid profiles. An explicit attempt to run the invalid profile fails; it must not fall back to a similarly named/global profile.
- [ ] Resolve local output-schema references within the approved package/project boundary. Disable network schema fetching and bound size, depth, and recursive references.
- [ ] Make reload atomic: validate a new catalog snapshot before publication. Existing runs retain their snapshot; revocation remains immediate.
- [ ] Add model/route/effort/spawn/output behavior to the actual worker launch path, not only to displayed metadata.

### Files

**Extend:** `crates/jcode-base/src/agent_profile.rs`; `crates/jcode-base/src/provider/agent_roles.rs`; current swarm admission/worker construction modules reached from `crates/jcode-app-core/src/server/swarm.rs`; configuration types; profile rendering in `crates/jcode-tui/src/tui/app/inline_interactive/agent_models.rs`.

### Exit gate

A user can define two profiles using different approved model routes and output contracts, invoke both through real swarm execution, and inspect their actual settings. Legacy profiles still use the configured swarm model unless explicitly migrated. Parent settings remain unchanged.

---

## 8. S3 — Reliable composition and verified multi-agent results

**Goal:** Improve the quality of completed work, not the number of simultaneously running agents.

### Tasks

- [ ] Integrate profile selection with the existing task DAG and worker ownership model. Do not create a parallel scheduler.
- [ ] Enforce child-profile allowlists, root concurrency, cumulative budgets, and configured depth at admission. Detect dependency cycles and impossible capability requirements before spawning.
- [ ] Emit typed `AgentResult` objects with separate execution, schema-validation, and task-verification states.
- [ ] Validate structured output strictly when requested. Permit only bounded repair attempts. Exhaustion becomes `schema_violation`, not success with an apology or malformed JSON.
- [ ] Require evidence for completion claims: verifier command/result references, artifact hashes, or deterministic state checks. Do not execute a worker-suggested verification command without normal policy admission.
- [ ] Extend existing lease/heartbeat/recovery mechanisms only where the census finds gaps. A reassignment fences late results from the former worker.
- [ ] Distinguish “worker finished,” “result accepted,” and “task verified.” A green child transcript alone cannot mark the parent task complete.
- [ ] Support explicit isolated worktrees for write-capable parallel tasks. Reuse existing worktree/file-touch facilities where present.
- [ ] Accept a worker patch only against its recorded base revision. Detect conflicting edits, renames, deletes, binary files, dirty parent files, and nested repository boundaries before integration.
- [ ] Use atomic patch acceptance where feasible. A conflict leaves parent files intact and returns a typed recovery requirement; never resolve a conflict by silently choosing the last writer.
- [ ] Keep shared-workspace mode only as an explicit, documented mode with current write guards. It must not be presented as equivalent to isolation.
- [ ] Propagate cancellation through the owned subtree and release reservations/leases. Unknown external effects remain unknown until checked.

### Test tasks

Use small realistic tasks: an API change plus frontend consumer; migration plus rollback validation; two workers editing adjacent lines; worker crash after writing an artifact; stale worker reporting after reassignment; schema-valid but semantically wrong output; a child unable to use its selected provider; a parent with pre-existing uncommitted changes.

### Files

**Extend:** existing swarm/task-DAG and completion code under `crates/jcode-app-core/src/server/`; `crates/jcode-app-core/src/tool/communicate*`; current session persistence; `crates/jcode-protocol/src/`; SDK result translation; existing file-write/patch guards.

**Proposed additions:** focused result-validation and patch-acceptance modules beside existing orchestration code, only where equivalent abstractions do not already exist.

### Exit gate

Composed tasks finish with verifiable outputs under bounded budgets. Failure, partial success, cancellation, and uncertain recovery are distinguishable. Conflict fixtures produce zero silent parent-workspace damage. New multi-agent behavior must beat or match the single-agent baseline before becoming a default.

---

## 9. S4 — Advisor lifecycle, context, and recovery hardening

**Goal:** Improve an already functioning investigative advisor. Preserve ongoing observation, retained exchanges, named advisors, cancellation, and existing delivery semantics. [S3][S6][S19][S20]

### 9.1 Model-aware context maintenance

- [ ] Replace byte-only planning with provider/model-aware token budgeting where supported; retain conservative byte limits as a separate resource guard.
- [ ] Account for system instructions, tool schemas, pending update, retained history, and reserved output tokens—not just the transcript body.
- [ ] Preserve coherent tool-call/result exchanges and model-private signed blocks on the same route. Do not forward incompatible native blocks after a model switch.
- [ ] Retain the current task requirements, latest explicit user changes, unresolved findings, and compact evidence references. Historical objectives must not override newer user instructions.
- [ ] Introduce explicit maintenance records: what was omitted, why, and how to reacquire evidence. Prevent silent truncation from becoming evidence of success.
- [ ] Reacquire changed files or diagnostics using bounded approved reads when an old finding matters again.
- [ ] Make summarization optional and budgeted. Test fidelity against deterministic facts; do not let a lossy summary replace a verifier or invent completed work.
- [ ] Keep automatic context promotion off unless an approved model-recovery chain explicitly permits it.

**Specific baseline:** `history.rs` currently bounds retention by complete exchanges and bytes. Extend that implementation instead of replacing it with full transcript replay. [S6]

### 9.2 Delivery and evidence lifecycle

- [ ] Model states explicitly: received, validated, deferred, queued, delivered, acknowledged, dismissed, superseded, quarantined, abandoned.
- [ ] Verify mid-turn nit/concern deferral and deterministic flushing. Blockers use permitted safe-boundary steering; normal incomplete intermediate work does not trigger completion criticism.
- [ ] Preserve terminal concerns as visible findings without forcing restatement. Give late blockers only bounded corrective continuation, and make incomplete drain visible.
- [ ] Maintain interruption immunity only after actual delivery. A newly observed blocker can bypass a concern cooldown, but repeated identical blockers cannot create an infinite loop.
- [ ] Combine concern identity with owner, invocation, evidence revision, and normalized issue location. Model-provided IDs are hints, not trusted global identifiers.
- [ ] Deduplicate across advisors conservatively while retaining attribution. Two different bugs in the same file must not collapse into one finding; agreement is not independent verification.
- [ ] Revalidate or mark stale findings when implicated content changes. Preserve useful unresolved findings across context maintenance without replaying old interrupts after restart.
- [ ] Validate the complete proposed advisor output before tool dispatch/publication where possible. Preserve schema, available-tool, and provenance checks; quarantine malformed/unavailable-tool output.
- [ ] Render advisory content as escaped, attributed evidence rather than privileged instructions. Regex-based dangerous-text filtering may supplement controls but is not a prompt-injection boundary.
- [ ] Add semantic tests for quoted dangerous commands versus newly proposed execution. Advice cannot acquire authority by quoting another agent or the user.

### 9.3 Recovery, budgets, and health

- [ ] Distinguish enabled, idle, reviewing, deferred, rate-limited, budget-exhausted, unsupported-route, failed, and stopped states with reasons.
- [ ] Retry bounded transient failures on the same approved route using cancellation-aware delay and provider retry hints.
- [ ] Reject deterministic request/model incompatibility without repeated identical calls.
- [ ] Add an optional user-approved recovery chain with exact routes, limits, and visible transition history. No project or extension can authorize it implicitly.
- [ ] On an allowed model switch, create fresh model-private context and seed only permitted visible task state. Do not transfer hidden reasoning or incompatible signatures.
- [ ] Bound per-advisor backlog, per-update work, total root supervision spend, and corrective continuations. Keep final-completion updates from being starved by intermediate observations.
- [ ] Apply fair admission across named advisors. One failing or slow advisor cannot starve its siblings or hold the primary's completion indefinitely.
- [ ] Persist budget exhaustion and dispositions without retaining raw secrets. Disable/re-enable, reconnect, or history rewind must not silently replenish a session budget.
- [ ] Surface actual useful interventions, suppressed noise, unavailable evidence, and incremental cost. “Enabled” must never imply “currently supervising.”

### Files

**Extend:** `crates/jcode-app-core/src/advisor/{history,runtime,delivery,suppression,evidence,investigation,model_selection,routing,persistence}.rs`; `advisor/roster/`; `agent/advisor_live.rs`; relevant turn/completion integration; `server/advisor_control.rs`; `crates/jcode-config-types/src/advisor.rs`; protocol, SDK, and advisor UI translations.

### Exit gate

A seeded bug is observed, independently investigated, delivered, and corrected without a second user prompt; clean tasks finish quietly; repeated faults remain bounded; cancellation never produces surprise continuation. Live quality/cost improvements must be measured separately from passing deterministic lifecycle tests.

---

## 10. S5 — Unified configuration and agent controls

**Goal:** Evolve the existing `/agents` interface into a consistent control surface. It already exists and already lists roles and custom profiles. [S4][S5]

### Tasks

- [ ] Add a common read/control API for built-in roles, named profiles, and named advisors, backed by the actual runtime rather than independent TUI assumptions.
- [ ] Show configured default versus currently resolved model, provider/runtime/account label, effort, source scope, effective tools, budgets, health, and pending changes.
- [ ] Clearly label inherited values. Selecting “inherit” means a defined resolution rule, not an empty string interpreted differently by each role.
- [ ] Preserve `/advisor` and current `/agents <role-or-profile>` behavior. Add new inspect/validate/edit actions only after checking existing command grammar and name collisions.
- [ ] Keep reserved built-in commands ahead of skill/extension resolution. Neither a profile nor plugin may shadow `/advisor`, `/agents`, or other control commands.
- [ ] Make model selection use the authenticated server catalog with exact route identity and supported reasoning effort.
- [ ] Distinguish user/project/session scope and the effective execution boundary before saving. A remote/SSH client must never write its local configuration while implying it changed the remote agent.
- [ ] Add server-authorized scope writes, validation, compare-and-swap config revisions, atomic replacement, and useful diagnostics. Two clients editing one profile must not silently overwrite each other.
- [ ] Preserve formatting/comments where supported by the chosen editor. Provide preview/backup for migrations that cannot round-trip losslessly.
- [ ] Keep unsaved input and selections across timeouts. Correlate late replies by request, session, and generation; do not apply them to a newly selected session.
- [ ] Add capability negotiation for older clients/servers. Unsupported controls return a clear compatible error rather than breaking deserialization or connection state.
- [ ] Expose equivalent headless/SDK control operations. Local standalone modes without the required runtime must report that limitation; do not emulate unsafe partial control in the UI.
- [ ] Add a diagnostics view for “why this model,” “why this tool is denied,” “why this advisor is idle,” and “what changed after reload.”

### Files

**Extend:** `crates/jcode-tui/src/tui/app/inline_interactive/agent_models.rs`; `advisor_picker.rs`; existing command dispatch and picker modules; `crates/jcode-app-core/src/server/`; `crates/jcode-protocol/src/`; `crates/jcode-harness-api*/src/`; `crates/jcode-sdk/src/`; `sdk/typescript/src/`; `src/cli/`.

### Exit gate

From a fresh supported session, a user can configure a named agent and advisor, select distinct routes/efforts, inspect effective permissions, survive reconnect, and verify that the next invocation used the displayed settings. All clients agree on scope and active configuration. Existing commands remain backward compatible.

---

## 11. S6 — A bounded extension platform

**Goal:** Cover real extension workflows without introducing arbitrary in-process code or a second policy system.

Current skills and their global/project overlays are an existing foundation. Reuse them. OMP's plugin command surface is a reference for user workflows, not a reason to duplicate every internal mechanism. [S8][S18]

### 11.1 Separate two extension classes

| Class | Contents | Activation model |
|---|---|---|
| Declarative package | Profiles, skills, prompt templates, output schemas, command metadata, and approved connection references. | Validate and install without executing package code. |
| Executable contribution | Custom tool implementation or event handler through a versioned subprocess protocol. | Explicit trust/permission approval; supervised lifecycle; sandbox only where genuinely supported. |

A subprocess is crash isolation, **not** a filesystem/network sandbox. Do not market an unsandboxed executable extension as safe for untrusted code. On hosts without an approved confinement backend, untrusted executable packages must remain disabled; explicitly trusted extensions have a clearly displayed broader trust requirement.

### 11.2 Package and host tasks

- [ ] Define one versioned package manifest with ID/version, supported host protocol, declared contributions, required permissions, and content digests.
- [ ] Support deterministic local-directory/package installation first. Add remote retrieval only through an explicit user-authorized install action with pinned content verification.
- [ ] Reuse the skill/profile registries and MCP connection infrastructure where appropriate. Package installation must not create another execution registry that bypasses normal tool policy.
- [ ] Prevent archive traversal, escaping links, oversized archives, identifier collisions, and reserved-command shadowing. Do not execute install scripts.
- [ ] Record a lockfile/install inventory and atomic enable/disable/update state. Capability increases require fresh approval; a signature alone is not a safety grant.
- [ ] Add namespaced tool registration with bounded argument/result schemas and operation-level capability metadata.
- [ ] Default executable calls to serialized unless the extension explicitly declares and passes reentrancy tests. Carry cancellation, deadlines, request IDs, generation, and owner identity.
- [ ] Add ordered event subscriptions with bounded queues. Observational handlers cannot change tool permissions or rewrite authoritative user instructions.
- [ ] Define failure policy per event type: observational handler failures must not stall the main loop; an explicitly configured policy gate fails according to its declared fail-closed contract.
- [ ] Keep event schemas versioned and redact/minimize data before delivery. Extensions receive only the session/workspace fields they were granted.
- [ ] Use a sanitized environment, approved working directory, explicit executable/argument arrays, and mediated secret references. Do not leak daemon credentials through inherited environment variables or unrestricted home access.
- [ ] Where claiming sandbox confinement, test actual filesystem, network, descendant-process, and credential boundaries on each supported OS. Otherwise label the mode trusted-execution only.
- [ ] On crash, timeout, unload, or cancellation, fence stale responses and terminate owned process groups. Bound restart attempts and clean up handles, pipes, subscriptions, and reservations.
- [ ] Revoke future capabilities immediately on disable. Keep uncertain external side effects visible; unloading is not rollback of changes an extension already made.
- [ ] Add host protocol negotiation so old extensions can fail clearly rather than destabilize the daemon.

### Files

**Existing seams:** `crates/jcode-base/src/skill.rs`; `agent_profile.rs`; existing MCP/hook configuration; `crates/jcode-app-core/src/tool/{mod,mcp,mcp_registration}.rs`; server lifecycle and tool dispatch; config/protocol/SDK surfaces.

**Proposed modules:** `crates/jcode-base/src/extensions/` for discovery/manifest/install state; `crates/jcode-app-core/src/extensions/` for host lifecycle and adapters. Confirm existing equivalents before creating them. Keep runtime code out of contract crates.

### Exit gate

Reference packages add an agent profile, skill, command, custom tool, and observational handler without modifying jcode core. Crashing or disabling a package does not hang the daemon or leak authority. Unsupported confinement is explicit. Default startup does not eagerly load executable extensions.

---

## 12. S7 — Authoring, compatibility, and reference packages

**Goal:** Make the extension platform usable and prove the selected OMP-style workflows—not merely expose internal APIs.

### Tasks

- [ ] Provide documented scaffolding and validation for a profile/package, using one manifest source of truth. CLI grammar is additive and must pass the S5 collision census.
- [ ] Publish a small versioned subprocess SDK/contract plus deterministic local test utilities. Avoid coupling authors to private Rust daemon types.
- [ ] Build an explicit OMP import tool with preview and a machine-readable compatibility report. Never import automatically at startup.
- [ ] Map only semantics that have passing compatibility fixtures. Treat `blocking`, spawn policy, model selectors, output schemas, and advisor instructions as substantive behavior, not cosmetic metadata.
- [ ] Report unsupported executable extension APIs without pretending arbitrary OMP TypeScript code can run unchanged.
- [ ] Preserve user-authored instructions and provenance; never import credentials, personalized endpoints, local absolute paths, or secret-bearing config.
- [ ] Ship a small curated reference set: read-only security reviewer, test-planning agent, architecture-review agent, deterministic verification tool, and one bounded observational integration.
- [ ] Bind examples to user-configured roles or catalog selections, not hardcoded live model names/accounts. Dangerous tools remain explicit opt-ins.
- [ ] Add uninstall/rollback tests and config round-trip examples. Ensure packages do not leave stale tools or commands after removal.
- [ ] Run predeclared authoring tasks with a developer unfamiliar with the implementation where possible. Record success, edits to core, setup steps, and recovery—not subjective “ease” alone.

### Proposed files

Use the established documentation/examples layout. Candidate additions are `docs/extensions.md`, `docs/agent-profiles.md`, and `examples/extensions/`; extend existing equivalents if present. Keep compatibility fixtures in the existing evaluation/test structure rather than a separate scoring framework.

### Exit gate

A new developer can build, validate, install, run, disable, and remove the reference contributions from the documentation. Supported OMP imports preserve behavior under tests; unsupported features are explicit. This supports a workflow-level extensibility claim, not a claim to have replicated OMP's entire ecosystem.

---

## 13. S8 — Prove the result and release safely

### 13.1 Deterministic and integration gates

- [ ] Run the existing repository guardrails with no new warnings or ratchet increases.
- [ ] Run all new contracts plus the pre-existing advisor, provider, session, swarm, write-integrity, LSP/DAP, and SDK regressions affected by the change.
- [ ] Exercise the actual freshly built daemon using a private home/workspace/socket. Record both client and server build identities; a new binary connecting to an old shared daemon is invalid evidence. [S1]
- [ ] Test upgrade and rollback from the pinned baseline with existing profiles, role settings, advisor checkpoints, and active sessions.
- [ ] Repeatedly create/cancel/reconnect/reload/dispose sessions and extension processes. Check task counts, file descriptors, memory, children, and persisted state for leaks.
- [ ] Test release/updater behavior on the fork channel. Preserve no-update controls and do not let validation replace the user's shared or stable binary.

### 13.2 Live comparative campaign

Run the S0-locked matched configuration and native-configuration tracks independently. Publish both successes and failures, confidence intervals, raw redacted result references, tool/cost accounting limits, and exact binary/model identities. Count timeouts and infrastructure failures in user-visible reliability; classify them additionally for diagnosis.

Include ablations: jcode without advisor, with one advisor, and with multiple advisors; single-agent versus composed execution. A complex configuration that performs worse than the simpler one should not become the default merely because it is available.

Re-run the pinned OMP baseline and optionally the latest OMP head as a **separate** campaign. Do not move the competitor revision mid-campaign. Never call a result permanent.

### 13.3 Resource and compatibility floors

Preserve the existing roadmap's floors unless a separately reviewed baseline correction changes them: no more than 5% startup/one-session memory regression and 10% incremental multi-session memory regression. Use repeatable host configurations and confidence-aware measurements, not one noisy sample. [S2]

For new facilities, default-disabled executable extensions must do no eager spawning; disabled advisors must not collect unnecessary evidence. Record tokens/cost of context maintenance and supervision explicitly. UI status must not hide exhausted budgets or stalled observers.

### 13.4 Lean CI implementation

Add tests to the current CI quality, relevant build/test, and existing acceptance jobs. Cache builds and avoid running identical expensive suites under several workflows. Keep deterministic tests mandatory on PRs. Run provider-billed comparisons only through a budgeted explicit/manual campaign; do not spend credentials from an untrusted PR or make paid live runs necessary for every commit.

Do not delete unique coverage, lower thresholds, or rebaseline failures simply to make the PR green. Workflow consolidation and genuine baseline repairs are separate, reviewable changes. [S9]

### 13.5 Final acceptance and release

All four targeted categories need their registered evidence before claiming “jcode surpasses OMP in the remaining weak areas.” A category that remains tied or unproven stays open.

Produce a redacted campaign report, migration guide, known-limitations list, rollback procedure, and exact final-head CI evidence. Release promotion requires explicit approval. Roll back on unauthorized continuation, wrong account/model execution, secret exposure, data loss, schema-invalid success, or material unexplained regression—regardless of the aggregate score.

---

## 14. Pull-request sequence

Each PR includes its behavioral objective, source/test mapping, compatibility impact, actual validation results, and rollback note. “Depends on” refers to merged contract changes or an explicitly reviewed integration base—not copying another agent's unapproved branch.

| PR | Scope | Depends on | Acceptance evidence |
|---|---|---|---|
| 01 | Gap census, baseline corrections, locked campaign contract. | None | No duplicate work; both adapters/configurations validated. |
| 02 | Resolved specs and immutable execution snapshots. | 01 | Route/effort/profile preservation across lifecycle paths. |
| 03 | Common capability intersection and revocation. | 02 | Nested/provider-hosted/delegated bypass fixtures denied. |
| 04 | Generation fencing, durable delivery state, shared reservations. | 02, 03 | Crash/race/cancellation and overspend-admission fixtures. |
| 05 | Versioned profile parser, diagnostics, legacy migration. | 02, 03 | Existing profiles unchanged; semantic imports not silently ignored. |
| 06 | Profile-specific routes, effort, spawn/output declarations at launch. | 04, 05 | Real child invocation uses its resolved profile contract. |
| 07 | Strict result validation and verified task acceptance. | 06 | Invalid/unverified output cannot complete a task. |
| 08 | Worktree/patch acceptance and composed-worker recovery. | 07 | Conflict/stale-worker/dirty-parent fixtures preserve data. |
| 09 | Advisor token-aware context and explicit maintenance. | 04 | Coherent histories; changing objectives; bounded recovery. |
| 10 | Advisor finding lifecycle, delivery, and evidence validation. | 09 | Deferred/terminal/escalation/cancellation matrix. |
| 11 | Advisor health, fair budgets, bounded opt-in recovery. | 10 | One failure cannot starve supervision or change routes silently. |
| 12 | Shared control API, scopes, revisions, capability negotiation. | 06; 11 for advisor fields | Remote authorization and old-client compatibility. |
| 13 | Existing `/agents` and `/advisor` UI convergence. | 12 | Fresh-user, empty-catalog, reconnect, and scope tasks. |
| 14 | CLI/SDK/ACP control parity and migration UX. | 12, 13 | Same effective settings through every supported client. |
| 15 | Declarative extension packages and install inventory. | 05, 06 | No install-time execution; atomic enable/disable; bounded extraction. |
| 16 | Namespaced tool/event contributions through common policy. | 03, 04, 15 | No command shadowing, cross-session access, or bypass path. |
| 17 | Supervised executable host and explicit trust/confinement modes. | 16 | Crash/timeout/unload/process cleanup and OS-specific boundary tests. |
| 18 | Author SDK, import preview, compatibility fixtures. | 07, 17 | Supported imports preserve semantics; unsupported behavior reported. |
| 19 | Reference packages, documentation, authoring-task validation. | 18 | Selected workflows completed without core edits. |
| 20 | Upgrade/rollback, soak, resource floors, final CI integration. | 08, 11, 14, 19 | Final-head deterministic, compatibility, and performance evidence. |
| 21 | Locked comparative campaign and release-readiness report. | 20 | Four category results with uncertainty, failures, and limitations. |

**Recommended first implementation slice:** PR 01, then PR 02. The first user-visible feature is PRs 05–06: richer existing profiles with independent approved routes. Do not begin with a plugin marketplace or a replacement advisor loop.

PR 04 may be split further if the current persistence/budget census reveals several independent state machines. PR 21 can be a documentation/results PR; it must not quietly modify the scorer to manufacture a win.

---

## 15. Cross-cutting regression matrix

| ID | Scenario | Required outcome |
|---|---|---|
| R01 | Legacy profile without model metadata. | Retains old swarm-model behavior and tools/effort semantics. |
| R02 | Legacy profile contains previously ignored OMP model fields. | Warn/preview migration; no surprise route activation. |
| R03 | One malformed project profile. | Unrelated profiles remain available; the named invalid one fails visibly. |
| R04 | Same profile name in two workspaces on one daemon. | Each session resolves its own project definition. |
| R05 | Two profiles choose different routes/efforts. | Independent authenticated conversations; parent unchanged. |
| R06 | Pinned route becomes unavailable. | Visible failure or approved recorded recovery; no silent account switch. |
| R07 | Tool permission revoked after prompt generation. | Execution is denied at dispatch. |
| R08 | Batch/nested/provider-hosted tool requests an effect. | Same role/session ceilings apply; no hidden backdoor. |
| R09 | Profile attempts recursive or unauthorized spawning. | Rejected or bounded by admitted policy and root budget. |
| R10 | Worker returns malformed or schema-valid but wrong output. | Schema and task verification states remain distinct; no false completion. |
| R11 | Late worker result arrives after reassignment/cancel. | Fenced; cannot modify task or workspace state. |
| R12 | Worker patch conflicts with dirty parent files. | No silent overwrite; recoverable conflict result. |
| R13 | Advisor receives “continue” after a long task. | Retains relevant requirements without reviving superseded instructions. |
| R14 | Advisor context maintenance near model limit. | Whole exchanges preserved; omissions explicit; no uncontrolled replay. |
| R15 | Concern arrives while work is intentionally incomplete. | Deferred or preserved appropriately; no unnecessary interruption. |
| R16 | Blocker arrives after final text but before finalization. | Bounded correction or explicit incomplete supervision; never an infinite loop. |
| R17 | Advisor repeats/rewords/escalates a finding. | Conservative dedupe; true escalation preserved; repeated blockers bounded. |
| R18 | User stops while provider/tool work is in flight. | No late auto-resume; owned cancellation/uncertain effects reported. |
| R19 | One named advisor stalls or exhausts quota. | Siblings and primary remain usable; degraded state visible. |
| R20 | Restart between note persistence and delivery. | Idempotent handling; no lost committed status or duplicate historical interrupt. |
| R21 | Two clients edit a configuration revision. | Conflict reported; no silent last-writer-wins change. |
| R22 | Remote/SSH client changes a model/profile. | Correct server scope; no misleading local-only write. |
| R23 | Old client connects after new protocol fields. | Negotiated support or compatible error; no connection failure. |
| R24 | Extension archive contains traversal or escaping symlink. | Install rejected before activation. |
| R25 | Extension attempts to shadow `/advisor` or `/agents`. | Rejected; built-in control dispatch preserved. |
| R26 | Extension crashes, floods events, or ignores cancellation. | Bounded queues/restarts; owned processes cleaned; daemon responsive. |
| R27 | Untrusted executable on a host lacking confinement. | Not activated; limitation explicit. |
| R28 | Extension asks for extra secrets/network/tools after update. | Fresh explicit grant required; previous authorization is insufficient. |
| R29 | Benchmark agent edits verifier/config or exploits its own scorer. | Locked trusted verifier remains authoritative; tampering recorded. |
| R30 | Repeated reload/dispose cycles with all features enabled. | No accumulating workers, handles, subscriptions, queues, or budget leaks. |

These are required scenarios, not a claim of exhaustive security coverage. Add specific regressions for every defect found during implementation without modifying the locked comparative holdouts.

---

## 16. Implementation handoff and completion checklist

### 16.1 Start safely

Use the actual fork default branch, `master`, not an assumed `main`:

```bash
git status --short
git remote -v
git fetch origin
git rev-parse origin/master
git worktree add ../jcode-remaining-gaps-s0 \
  -b feat/omp-remaining-gaps-baseline origin/master
```

Do not reset, stash, or discard unrelated work automatically. Read the applicable repository guidelines. If existing remote branches contain related work, inspect their status but integrate only under the repository's ownership/authorization rules. [S1]

### 16.2 Validation commands already documented by the repository

Run these from the appropriate owned worktree after installing the documented development dependencies:

```bash
scripts/check_guardrails.sh --skip-slow
JCODE_DEV_FEATURE_PROFILE=minimal scripts/test_fast.sh
python3 -m unittest discover -s scripts/competitive_eval/tests -t .
```

Before merge readiness, run the full gates and relevant integration suites:

```bash
scripts/check_guardrails.sh
scripts/test_e2e.sh
```

Use `scripts/dev_cargo.sh` for normal builds as directed by repository policy. For actual runtime acceptance, follow the current isolated-socket acceptance harness and verify its newly built server identity. Do not substitute compilation for daemon acceptance or fabricate a passing result when local sockets/providers are unavailable. [S1][S7]

### 16.3 Required PR evidence template

```text
Objective:
Base SHA / implementation SHA:
Requirements satisfied:
Existing behavior reused:
New or changed execution paths:
Compatibility and migration impact:
Security/trust boundary:
Tests actually run and results:
Tests not run and reason:
Client/server binary identity for runtime tests:
Measured cost/performance impact:
Known limitations:
Rollback procedure:
```

### 16.4 Program completion

- [ ] Existing declarative profiles, role pickers, advisor runtime, and evaluation harness were extended rather than duplicated.
- [ ] Per-profile routes, spawning, and structured results work end to end.
- [ ] Advisor quality and failure behavior meet the registered gates.
- [ ] Model/account/effort and configuration scope are consistent across supported clients.
- [ ] Extension workflows operate without core modifications and without hidden authority expansion.
- [ ] All mandatory deterministic/adversarial cases pass at the final head.
- [ ] Existing jcode data-integrity, provider, SDK, and resource floors remain satisfied.
- [ ] Both pinned products were evaluated under the same frozen rules.
- [ ] Every claimed category win has supporting results and uncertainty; ties remain labeled ties.
- [ ] Release/migration/rollback evidence is ready for explicit approval.

**Completion means verified behavior and comparative evidence—not a longer feature list, a green subset of tests, or a manually increased score.**

---

## 17. Sources

Source review used the immutable revisions at the top of this document. Repository plans are historical/design evidence; source code supports the implementation inventory. No full build, local runtime acceptance, or paid model campaign was performed while preparing this plan.

[S1]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/AGENTS.md "Repository boundaries, workflow, runtime identity, and guardrails"
[S2]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/docs/plans/omp-overtake/MASTER_PLAN.md "Existing OMP-overtake program and measurement rules"
[S3]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/docs/plans/omp-overtake/PHASE4_ADVISOR_AGENT_PLAN.md "Existing investigative-advisor implementation record and limits"
[S4]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/crates/jcode-tui/src/tui/app/inline_interactive/agent_models.rs "Existing role/profile picker and configured model behavior"
[S5]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/crates/jcode-base/src/agent_profile.rs "Existing declarative profile parser, discovery, and overlay"
[S6]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/crates/jcode-app-core/src/advisor/history.rs "Current retained-exchange and objective maintenance"
[S7]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/scripts/competitive_eval/README.md "Existing evaluation harness, isolation, and adapters"
[S8]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/crates/jcode-base/src/skill.rs "Existing skill registry and session-local project overlays"
[S9]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/.github/workflows/ci.yml "Existing CI and quality gates"
[S10]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/crates/jcode-base/src/provider/agent_roles.rs "Private route-pinned provider construction for roles"
[S11]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/crates/jcode-app-core/src/advisor/model_selection.rs "Advisor route validation and stale-work invalidation"
[S12]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/crates/jcode-app-core/src/advisor/investigation.rs "Investigative tools, workspace checks, policy hooks, and redaction"
[S13]: https://github.com/LLMpsycho/oh-my-pi/blob/8553cd681ad27014d05cf5b9708ef3322975a409/packages/coding-agent/src/advisor/runtime.ts "OMP advisor context and recovery behavior"
[S14]: https://github.com/LLMpsycho/oh-my-pi/blob/8553cd681ad27014d05cf5b9708ef3322975a409/packages/coding-agent/src/advisor/advise-tool.ts "OMP deferral, severity, deduplication, and delivery"
[S15]: https://github.com/LLMpsycho/oh-my-pi/blob/8553cd681ad27014d05cf5b9708ef3322975a409/packages/coding-agent/src/advisor/config.ts "OMP advisor configuration and tool grants"
[S16]: https://github.com/LLMpsycho/oh-my-pi/blob/8553cd681ad27014d05cf5b9708ef3322975a409/packages/coding-agent/src/task/types.ts "OMP task/output schemas and isolation options"
[S17]: https://github.com/LLMpsycho/oh-my-pi/blob/8553cd681ad27014d05cf5b9708ef3322975a409/packages/coding-agent/src/cli/agents-cli.ts "OMP exported agent frontmatter"
[S18]: https://github.com/LLMpsycho/oh-my-pi/blob/8553cd681ad27014d05cf5b9708ef3322975a409/packages/coding-agent/src/cli-commands.ts "OMP plugin and authoring command surfaces"
[S19]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/crates/jcode-app-core/src/advisor/runtime.rs "Current investigative loop, coalescing, and advise schema"
[S20]: https://github.com/LLMpsycho/jcode/blob/25b9f90f4be17d90289754681155cc21d562f592/crates/jcode-app-core/src/advisor/delivery.rs "Current cancellation, terminal delivery, and immunity behavior"
