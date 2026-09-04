# scripts — Build, Test, and Budget Gates (No Manifest)

## OVERVIEW

Unpackaged `bash`/`python3`/`pwsh` automation (164 files): build wrapper, test orchestrators, ratchet budgets, repro harnesses. No `package.json`/`Cargo.toml` — run directly.

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Full quality gate | `check_guardrails.sh` | fmt, check, clippy, budgets, machete, parity |
| Fast unit loop | `test_fast.sh` | `--lib --bin jcode` + startup budget if release exists |
| E2E run | `test_e2e.sh` (+ `real_provider_smoke.sh`, `test_auth_e2e.sh` behind env gates) | lib suites + `--test e2e` |
| Build wrapper | `dev_cargo.sh` | memory-sized jobs, host flock, linker, `JCODE_BUILD_GIT_HASH/DATE` |
| Budget ratchets | `check_*_budget.{sh,py}` + `*_budget.{txt,json}` | warnings(0), >1200-LOC sizes, panic, swallowed-error, wildcard |
| Security preflight | `security_preflight.sh` | triaged `RUSTSEC-*` ignores for lettre/rustls-webpki/lopdf/quick-xml; all else fails |
| SDK packaging | `test_sdk_*.sh` | published-tarball install+import checks |
| Repro cases | `repro/<name>/` (own `Cargo.toml`) | isolated crates, e.g. `tls-bad-record-mac` |

## CONVENTIONS

- Name by role: `test_*`, `bench_*`, `repro_*`, `check_*_budget`, `benchmark_*`; orchestrators are `test_fast.sh` / `test_e2e.sh` / `test_ci_suites.py`.
- Budget scripts support `--update` to rebaseline — only when the narrowing is real, never to silence growth.
- Shared helpers live in `lib/` (`configure_path.sh`); keep top-level scripts executable with explicit shebangs.
- `security_preflight.sh` also scans secret patterns (`AKIA/ASIA`, `gh*_`, `xox*`, `AIza`, private-key headers) and rejects world-writable `scripts/`.

## COMMANDS

```bash
scripts/check_guardrails.sh --fix        # fmt + rebaseline ratchets (only for real narrowing)
scripts/test_fast.sh                      # default profiles; prefix JCODE_DEV_FEATURE_PROFILE=minimal for probes
```
