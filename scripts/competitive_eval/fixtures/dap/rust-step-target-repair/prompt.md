Use the DAP debugger to repair the incorrect result in this Rust program.
Launch `target/debug/dap-step-target-repair`, stop on the nested-call line in
`main`, request step-in targets for that frame, and use a targeted step to
inspect the call chain. Fix only the erroneous transformation so the program
prints `42`. Preserve `read_seed`, `scale`, `finalize`, and the nested call. Do
not hardcode the final output, add diagnostic prints, or modify the task,
prompt, setup, or verifier files.
