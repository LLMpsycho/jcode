Use the DAP debugger to reproduce and localize the panic in this Rust program.
Launch `target/debug/dap-crash-localization`, stop inside `select_label`, and
inspect the relevant frame and variables before changing code. Make the
smallest source fix so slot 2 prints `gamma`. Do not hardcode the final output,
remove the helper, add diagnostic prints, or modify the task, prompt, setup, or
verifier files.
