# Competitive evaluation scorecard

Deterministic task success is primary. Aggregate weights: correctness 45%, autonomous completion 15%, tool reliability 10%, verification 10%, token efficiency 8%, latency 5%, resource use 4%, and safety 3%.

## Required floors

- Zero silent stale edits.
- No safety regression.
- At least 95% of OMP success in every major category before an overall win claim.
- Startup and single-session memory regression at most 5%.
- Incremental multi-session memory regression at most 10%.
- No unbounded output, loops, or leaked processes.

A valid claim names both Git SHAs, binary hashes, campaign ID, provider, model, effort, and fixture/verifier digests.
