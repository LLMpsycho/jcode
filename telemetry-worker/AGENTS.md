# telemetry-worker — Cloudflare Telemetry Ingest (D1 + R2 + Analytics Engine)

## OVERVIEW

Private `jcode-telemetry` Worker: single `src/worker.js` entry behind `wrangler.toml` with D1 (`jcode-telemetry`), R2 (`TRANSCRIPTS`), Analytics Engine datasets, and a daily cron. No Rust involved.

## STRUCTURE

```
telemetry-worker/
├── src/worker.js        # sole entry (wrangler main)
├── migrations/00*.sql  # D1 migrations (per-change files, e.g. 0025_transcript_uploads)
├── *.sql               # dashboards: dau/users/geo/health/token-value
├── scripts/            # run-dashboard.mjs, sync-model-prices.mjs
└── test/               # worker.test.mjs, token-value.test.mjs (node --test)
```

## CONVENTIONS

- Schema: never add columns to `events` — extend the detail tables instead.
- Transcript access is a production-data operation; do not expose the R2 bucket.
- Dashboards are versioned `*.sql` at the top level; ops run through `wrangler d1 execute` npm scripts (`migrate:*`, `health/dau/users/geo/conversion`).
- Custom domains: `telemetry.{solosystems.dev,jcode.sh}`; compat date pinned at `2025-01-01`.

## COMMANDS

```bash
npm run dev                # workdir telemetry-worker: wrangler dev
npm test                   # node --test test/*.test.mjs
npm run health             # dashboard query via scripts/run-dashboard.mjs
npm run migrate:<name>     # per-file D1 migration, --remote (see package.json scripts)
```
