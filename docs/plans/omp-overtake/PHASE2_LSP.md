# Phase 2 shared LSP implementation

This branch implements the Rust-first Phase 2 path from the OMP overtake master plan.

## Implemented

- `jcode-lsp-types` with JSON-RPC, source location, diagnostic, edit, and semantic verification contracts.
- `jcode-lsp` with bounded Content-Length framing, an asynchronous multiplexed client, cancellation, timeouts, process lifecycle management, bounded stderr, strict executable discovery, and a fake in-memory server.
- Strict `[lsp]` configuration with documented defaults and unknown-field rejection.
- A persistent-server-owned pool keyed by canonical root, worktree identity, server id, and configuration digest.
- Controlled language-server environments that do not forward provider API credentials.
- Document open/change/save synchronization with UTF-16 incremental ranges.
- Push and pull diagnostic support, including new/worsened diagnostic deltas.
- Agent actions for status, diagnostics, hover, definition, references, document/workspace symbols, implementation, type definition, signature help, call hierarchy, code actions, capabilities, reload, and symbol rename.
- Bounded human-readable output plus structured metadata. Raw protocol payloads are not emitted by default.
- Same-turn semantic feedback on all built-in mutation tools that emit revision metadata.
- Cross-file Rust rename preview and atomic apply. Apply requires current full-file reads for every affected file, revalidates all files before publication, preserves permissions, updates the snapshot ledger, rolls back publication failures, and emits file-touch events.
- Graceful missing-executable behavior and process-group termination on shutdown.

## Configuration precedence

The currently implemented path uses Jcode's existing configuration load order:

1. built-in `LspConfig` defaults;
2. the loaded user configuration file.

The LSP section rejects unknown keys and invalid types. Project-specific LSP files and explicit per-session LSP overrides are not implemented yet, so this branch does not claim the full four-layer precedence described as the preferred end state in the master plan.

## Verified behavior

- Framing is tested at every byte split, with multiple frames, malformed headers, and payload limits.
- The fake server verifies out-of-order responses, notifications, cancellation, timeout, server requests, and process exit.
- Installed `rust-analyzer` initializes and shuts down through the real process transport.
- Two callers with one workspace key reuse a server. Different worktree identities receive different servers.
- Explicit reload replaces only the selected pooled server.
- Real Rust definition lookup succeeds.
- A real introduced Rust type error is returned by negotiated pull diagnostics.
- A prewarmed write returns the introduced error on the same tool result.
- Real cross-file Rust rename returns edits for both definition and reference.
- The public `read` then `lsp rename apply` workflow updates both files atomically.
- A stale file rejects a multi-file rename before any requested file is changed.

## Remaining Phase 2 work

- Idle eviction and bounded exponential restart backoff.
- Hot-reload state restoration beyond clean process shutdown/restart.
- Applying selected code actions and file rename operations.
- TypeScript, Python, and Go server presets and integration suites.
- Project/session configuration overrides.
- SDK-specific typed rendering beyond generic tool metadata.
- Competitive OMP campaign measurements for the supported LSP task subset.
