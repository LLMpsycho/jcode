# Deterministic fixtures

`mock/` covers pass, verifier fail, agent timeout, agent crash, unsupported
capability, and capped large-output behavior without a model or competitor.
Every fixture is copied before execution and the source tree remains immutable.

`lsp/` covers semantic diagnostics and refactoring. `dap/` covers debugger-led
crash localization and targeted step-in navigation. DAP fixture setup requires
an existing `lldb-dap`; set `JCODE_EVAL_LLDB_DAP` to its absolute path when it
is not discoverable through `PATH` or `xcrun`.
