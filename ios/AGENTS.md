# ios — Native iOS App (SwiftPM + XcodeGen)

## OVERVIEW

Dual-manifest satellite: `Package.swift` (`JCodeKit` library, iOS17/macOS14, strict concurrency) + `project.yml` (XcodeGen `JCodeMobile` app depending on `JCodeKit` by path).

## STRUCTURE

```
ios/
├── Sources/JCodeKit/    # shared business logic + tests in Tests/JCodeKitTests
├── Sources/JCodeMobile/ # app shell/views (bundle com.jcode.mobile, Swift 6.0)
└── TestHarness/         # e2e: run_e2e.sh, check_production.sh, mock_gateway.py, protocol + UI lint/matrix scripts
```

## CONVENTIONS

- Shared logic belongs in `JCodeKit` (unit-tested via `swift test`); `JCodeMobile` stays shell/views.
- E2E goes through `TestHarness/` against `mock_gateway.py` — never point the harness at production without `check_production.sh`.
- Keep `Package.swift` and `project.yml` in sync when adding targets/files (SPM vs XcodeGen resolve independently).
- `DEVELOPMENT_TEAM TAS6ARKDN7` signs the app target; do not change bundle id or team without the release owner.
- UI lint/matrix/metrics scripts (`ui_lint|matrix|metrics.py`) run from `TestHarness/` alongside the e2e harness.

## COMMANDS

```bash
swift test                                     # workdir ios
./TestHarness/run_e2e.sh                       # workdir ios/TestHarness
./TestHarness/check_production.sh              # required before any prod-pointed run
```
