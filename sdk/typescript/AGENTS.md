# sdk/typescript — TypeScript SDK (`@1jehuang/jcode-sdk`)

## OVERVIEW

Compiled TS client over the harness Unix-socket bridge (`jcode api-bridge` ships in the release binary, so SDK users need no Rust toolchain). Only satellite with `package.json` + lockfile + `tsconfig.json`.

## STRUCTURE

```
sdk/typescript/
├── src/     # index.ts re-exports protocol/client/launch/binary (+ framing/sockets)
├── test/    # *.test.ts + live-*.mjs + mock-harness.ts
└── examples/demo-app/  # own package.json demo
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Client API change | `src/client.ts`, `src/index.ts` | keep `sideEffects:false`, `engines.node>=20` |
| Wire protocol | `src/protocol.ts`, `src/framing.ts`, `src/sockets.ts` | bridge speaks Unix sockets |
| Launch/binary | `src/launch.ts`, `src/binary.ts` | resolves platform bin from `sdk/npm/*` optionalDeps |
| Capability list | `test/*.test.ts` + `crates/jcode-sdk/src/sdk_tests/parity.rs` | `snake_case`↔`camelCase` parity is contract |

## CONVENTIONS

- `strict` TS (`ES2022/NodeNext`, `rootDir:src`→`dist`); never widen without reason.
- Capability parity with the Rust `JcodeClient` is enforced by `cargo test -p jcode-sdk parity` — change both sides in one task.
- Platform binaries come from the six `sdk/npm/*` shim packages (manifest-only, no code); version them with the SDK.

## COMMANDS

```bash
npm ci --no-audit --no-fund   # workdir sdk/typescript
npm run check                 # typecheck + build + node --test
bash scripts/test_sdk_package.sh  # run from repo root: tarball install+import
```
