/// Rendering the route catalog must never schedule network work.
///
/// Regression guard for the "spawning a session refetches every provider
/// catalog" bug: route building used to schedule a background `/models` fetch
/// for each stale or missing profile cache, so every session attach and picker
/// open fanned out dozens of HTTP requests. Refresh cadence now belongs solely
/// to the background catalog scheduler.
#[test]
fn building_direct_profile_routes_does_not_schedule_catalog_refreshes() {
    with_clean_provider_test_env(|| {
        let runtime = enter_test_runtime();
        runtime.block_on(async {
            crate::provider_catalog::save_env_value_to_env_file(
                "OPENROUTER_API_KEY",
                "openrouter.env",
                Some("sk-test-openrouter"),
            )
            .expect("save openrouter key");

            // Deliberately leave every profile cache missing/stale: under the
            // old behavior this was the worst case that scheduled a refresh
            // for each configured profile.
            jcode_provider_openrouter_runtime::reset_profile_catalog_refresh_tracker_for_tests();

            for profile in crate::provider_catalog::openai_compatible_profiles()
                .iter()
                .copied()
            {
                let _ = super::direct_openai_compatible_profile_routes(profile);
            }

            // If route building had scheduled refreshes, the profile tracker
            // would have recorded attempts, and this direct call for the
            // standard OpenRouter namespace would be throttled/in-flight.
            assert!(
                openrouter::maybe_schedule_standard_openrouter_catalog_refresh(
                    "unit test post-render scheduling"
                ),
                "route building must leave the refresh tracker untouched"
            );
        });
    });
}

/// The scheduler's staleness predicate must treat a missing or mismatched
/// cache as needing a refresh, so the sweeper actually populates cold caches.
#[test]
fn profile_catalog_cache_needs_refresh_for_missing_cache() {
    with_clean_provider_test_env(|| {
        let profile = crate::provider_catalog::openai_compatible_profiles()
            .first()
            .copied()
            .expect("at least one OpenAI-compatible profile is defined");
        assert!(
            super::catalog_scheduler::profile_catalog_cache_needs_refresh(profile),
            "a missing catalog cache must be reported as needing a refresh"
        );
    });
}

#[path = "private_session.rs"]
mod private_session;
