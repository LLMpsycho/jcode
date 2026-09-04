# DAP comparison baseline: Jcode vs oh-my-pi

**Captured:** 2026-09-04  
**Purpose:** Preserve a repeatable baseline for measuring Jcode's DAP progress against oh-my-pi.

## Compared revisions

- Jcode: `5232f4db5` (`master`), including the Phase 3 DAP implementation and the execution-revision fix found during this review.
- oh-my-pi: `e67e60f966` from `/Users/besi/Code/oh-my-pi`.
- Primary Jcode specification: [`../plans/omp-overtake/PHASE3_DAP.md`](../plans/omp-overtake/PHASE3_DAP.md).

The comparison is a point-in-time audit. Scores should be recalculated when either revision or the weighting model changes.

## Scoring model

Scores use a 0–10 scale and the following fixed weights:

| Category | Weight | Jcode | oh-my-pi |
|---|---:|---:|---:|
| Debugging capability breadth | 25% | 7.5 | 9.5 |
| Safety and agent authority | 20% | 9.7 | 7.0 |
| Protocol and lifecycle correctness | 20% | 9.4 | 8.4 |
| UX and product integration | 15% | 8.0 | 9.1 |
| Adapter ecosystem | 10% | 5.5 | 9.5 |
| Tests and engineering rigor | 10% | 9.3 | 8.2 |
| **Weighted score** | **100%** | **8.4** | **8.6** |

Weighted totals are calculated from the displayed category values and rounded to one decimal place. The unrounded totals are 8.375 for Jcode and 8.590 for oh-my-pi.

## Current verdict

oh-my-pi is slightly ahead overall because it has substantially broader debugger and adapter coverage. Jcode has the stronger safety, ownership, boundedness, stale-reference protection, and lifecycle foundation for autonomous-agent debugging.

### Jcode advantages

- No arbitrary caller-supplied PID attachment.
- No unrestricted remote host or port attachment.
- No unrestricted raw DAP request action.
- Canonical workspace containment for programs and source files.
- Opaque owner-, session-, frame-, and execution-revision-scoped tokens.
- Explicit evaluation opt-in and side-effect acknowledgement.
- Bounded protocol frames, queues, events, output, inspection results, and deadlines.
- Cancellation-safe transport and request cleanup.
- Owned process groups with descendant cleanup and escalation.
- Breakpoint source hashing, transaction serialization, event reconciliation, and indeterminate-state reporting.
- Extensive process, race, lifecycle, protocol, tool, SDK, and TUI tests.

### Jcode disadvantages

- Primarily limited to validated `lldb-dap` and native GDB DAP profiles.
- No arbitrary PID or remote-port debugging.
- No function, instruction, or data breakpoint surface yet.
- No memory read/write, disassembly, module, or loaded-source actions.
- Reverse `runInTerminal` requests are rejected rather than executed.
- No recursive child-debug-session support.
- Narrower debugger UI and adapter configuration experience.

### oh-my-pi advantages

- Twenty-eight public debug actions versus Jcode's eighteen-action surface.
- Function, instruction, and data breakpoints.
- Disassembly, memory read/write, modules, and loaded sources.
- Arbitrary custom DAP requests.
- Local PID and remote host/port attachment.
- Recursive child sessions through `startDebugging`.
- Reverse `runInTerminal` process spawning.
- Built-in and configurable support for GDB, LLDB DAP, CodeLLDB, debugpy, Delve, JavaScript, .NET, Kotlin, Ruby, PHP, Bash, Dart, Flutter, and Elixir adapters.
- Stdio, socket, and TCP adapter transports.
- Project-aware adapter selection using extensions and root markers.
- Rich debugging UI, log/SSE viewers, profiling, heap snapshots, diagnostics, and report bundles.

### oh-my-pi disadvantages

- Caller-supplied PID, host, and port increase agent authority and risk.
- Raw `custom_request` and memory writes have a larger damage radius.
- Adapter-native numeric frame and variable references are exposed to the model.
- Reverse requests can spawn processes under broad execution approval.
- One active root session tree.
- In-memory session state only.
- External adapter installation remains the user's responsibility.
- Runtime coverage is uneven across the full adapter and advanced-action matrix.

## Validation performed

### Jcode

The following public and integration boundaries passed after the review fix:

- `cargo fmt --all -- --check`
- Full `jcode-dap` package suite, including unit, process, subprocess, and doc tests.
- `jcode-app-core` server-owned DAP tool tests.
- Rust SDK client behavior tests.
- Jcode TUI DAP rendering and bounded-summary tests.

A focused real-process suite passed **13/13**, covering launch, owned attach, GDB-profile wiring, failed-start cleanup, disconnect escalation, target exit, and adapter/target process-group termination.

The full package run initially found a deterministic defect: `advance_execution` was called only inside `debug_assert!`, so release-like builds did not advance the execution revision. The call was moved outside the assertion and committed as:

- `5232f4db5 fix(dap): advance revision outside debug assertions`

The complete DAP and integration-boundary sequence passed after that fix.

### oh-my-pi

The repository-defined DAP configuration, launch-failure, transport-resilience, and multi-session suites passed **51/51**. These tests directly exercise the public `DebugTool` for launch and attach validation and cover adapter configuration, missing-adapter guidance, transport closure, recursive sessions, breakpoint routing, `runInTerminal` output, and termination.

The full twenty-eight-action inventory is source-confirmed. Not every advanced action was exercised end to end during this audit.

### External-adapter limitation

Neither `lldb-dap` nor `gdb` was available in `PATH` during the audit. Consequently, an end-user comparison against the same installed production adapter was not possible. Repository integration tests provide strong boundary evidence but do not replace a future cross-product production-adapter benchmark.

## Projected Jcode score after the full overtake plan

If Jcode adds safe polyglot adapters, richer read-only inspection, child sessions, and stronger debugging UX without weakening its ownership model, the projected score is:

| Category | Current | Projected |
|---|---:|---:|
| Debugging capability breadth | 7.5 | 9.0 |
| Safety and agent authority | 9.7 | 9.7 |
| Protocol and lifecycle correctness | 9.4 | 9.6 |
| UX and product integration | 8.0 | 9.1 |
| Adapter ecosystem | 5.5 | 8.5 |
| Tests and engineering rigor | 9.3 | 9.5 |
| **Weighted score** | **8.4** | **9.3** |

This would exceed the current oh-my-pi baseline of 8.6. The projected unrounded Jcode total is 9.275. It is a projection, not a verified outcome.

## Highest-value work before the next comparison

1. Add safe profiles for debugpy, Delve, CodeLLDB, and JavaScript debugging.
2. Add function breakpoints using opaque, revision-scoped references.
3. Add bounded modules and loaded-source inspection.
4. Add read-only disassembly and memory reads behind explicit capability and policy gates.
5. Support child debug sessions with strict workspace and process ownership.
6. Improve adapter discovery, setup diagnostics, and TUI workflows.
7. Keep arbitrary PID attach, memory writes, and unrestricted custom requests disabled by default.
8. Add a policy-gated advanced mode for trusted, explicitly approved workflows.
9. Install one common production adapter and run identical end-to-end debugging scenarios in both products.

## Recomparison procedure

For a later audit:

1. Record both exact Git revisions and whether either tree is dirty.
2. Keep the category weights unchanged unless the report explicitly introduces a new scoring version.
3. Run both repositories' DAP unit and integration suites.
4. Run identical production-adapter scenarios for launch, breakpoint, stop, stack, scopes, variables, stepping, evaluation, and cleanup.
5. Include failure scenarios: adapter exit, target exit, timeout, cancellation, stale references, oversized output, and owner disconnect.
6. Re-score each category using observed behavior, not feature presence alone.
7. Document score changes and their concrete evidence in a new dated audit rather than overwriting this baseline.
