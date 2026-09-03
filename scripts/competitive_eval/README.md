# Competitive evaluation harness

This standard-library-only Python package implements the deterministic Phase 0
runner described in `docs/plans/omp-overtake/MASTER_PLAN.md`. It validates the
locked JSON schemas and baseline hashes before every campaign, copies every task
into a fresh workspace, creates isolated home/config/temp/socket paths, enforces
deadlines on owned process groups, caps output, redacts artifacts, and writes
result JSON atomically.

## Deterministic smoke campaign

```sh
python3 -m scripts.competitive_eval.run_campaign \
  scripts/competitive_eval/fixtures/mock/*/task.json \
  --agents mock --seed 7 --campaign-id mock-smoke
```

The command intentionally exits nonzero because the fixture set includes fail,
timeout, crash, and unsupported outcomes. Generated data lives under
`competitive-eval/campaigns/` and is gitignored.

Validate configuration without either competitor installed:

```sh
python3 -m scripts.competitive_eval.run_campaign \
  scripts/competitive_eval/fixtures/mock/pass/task.json \
  --agents jcode,omp --dry-run --campaign-id dry-run
```

Run one trial, summarize a campaign, or compare paired results:

```sh
python3 -m scripts.competitive_eval.run_one TASK.json --agent mock --result-dir /tmp/eval-one
python3 -m scripts.competitive_eval.summarize competitive-eval/campaigns/CAMPAIGN
python3 -m scripts.competitive_eval.compare competitive-eval/campaigns/CAMPAIGN
```

## Live adapters

Jcode and OMP are never downloaded. Supply explicit executable paths with
`--jcode-binary` / `JCODE_EVAL_JCODE_BINARY` and
`--omp-binary` / `JCODE_EVAL_OMP_BINARY`. OMP defaults to
`OMP --print PROMPT`; set `JCODE_EVAL_OMP_ARGS` to a shell-style argument list
using exact `{prompt}`, `{prompt_file}`, or `{workspace}` placeholders when the
pinned OMP version uses a different noninteractive CLI.

Each adapter fingerprints the executable with SHA-256 and a bounded `--version`
probe. Missing binaries or capabilities produce `unsupported`, never an install
or a weakened task.

## Tests

```sh
python3 -m unittest discover -s scripts/competitive_eval/tests -t .
```
