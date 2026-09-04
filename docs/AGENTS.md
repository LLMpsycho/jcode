# docs — Reference Docs (Markdown Only, No Build)

## OVERVIEW

Hand-written reference; no manifest, toolchain, or tests. Location encodes freshness — read the path before trusting the content.

## STRUCTURE

```
docs/
├── *.md        # current behavior (architecture, features, system specs)
├── plans/      # forward-looking roadmaps/TODOs; may be partially implemented or stale
├── audits/     # point-in-time reviews; historical snapshots, never updated
├── proposals/  # uncommitted designs
└── dev/        # process and testing notes
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Crate boundaries | `CRATE_OWNERSHIP_BOUNDARIES.md`, `MODULAR_ARCHITECTURE_RFC.md` | normative for splits/moves |
| Server behavior | `SERVER_ARCHITECTURE.md` | daemon lifecycle reference |
| Swarm/memory | `SWARM_*.md`, `MEMORY_*.md`, `MEMORY_INCIDENT_RUNBOOK.md` | runbook before touching sessions |
| Refactor process | `REFACTORING.md` | shadow-env + phase-verify gates |
| Providers | `PROVIDER_DOCTOR.md`, `AWS_BEDROCK_PROVIDER.md` | live-probe contracts |

## CONVENTIONS

- Current behavior lives top-level; speculation goes in `plans/` or `proposals/`.
- Prefer updating an existing doc over adding a near-duplicate.
- Repo root holds meta files only (README, CONTRIBUTING, RELEASING, AGENTS, LICENSE); everything else lives here.
