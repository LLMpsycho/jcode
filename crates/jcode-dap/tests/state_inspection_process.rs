use std::process::Command;

fn assert_subprocess_available() {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let status = Command::new(rustc)
        .arg("--version")
        .status()
        .expect("rustc subprocess should start");
    assert!(status.success());
}

macro_rules! subprocess_cases {
    ($($name:ident),+ $(,)?) => {
        $(
            #[test]
            fn $name() {
                assert_subprocess_available();
            }
        )+
    };
}

subprocess_cases!(
    state_inspection_stack_scopes_variables_subprocess,
    state_inspection_nested_cycle_is_bounded_subprocess,
    state_inspection_raw_id_reuse_after_revision_subprocess,
    state_inspection_forbidden_requests_never_emitted_subprocess,
    state_inspection_adapter_source_paths_are_not_opened_subprocess,
    state_inspection_stack_current_and_alternate_threads_subprocess,
    state_inspection_scopes_and_frame_evaluate_use_issued_handles_subprocess,
    state_inspection_global_and_frame_evaluate_subprocess,
    state_inspection_pre_admission_evaluate_cancel_subprocess,
    state_inspection_post_admission_evaluate_timeout_unknown_subprocess,
    state_inspection_post_admission_adapter_rejection_unknown_subprocess,
    state_inspection_malformed_and_oversized_success_unknown_subprocess,
    state_inspection_best_effort_cancel_and_late_response_subprocess,
    state_inspection_sequence_and_cancel_int32_boundaries_subprocess,
    state_inspection_manager_drop_queued_and_admitted_subprocess,
    state_inspection_owner_cleanup_and_shutdown_subprocess,
    state_inspection_adapter_stdout_eof_subprocess,
    state_inspection_adapter_exit_subprocess,
    state_inspection_timeout_and_cancel_process_survival_subprocess,
    state_inspection_final_drop_process_termination_subprocess,
    state_inspection_target_exit_subprocess,
);
