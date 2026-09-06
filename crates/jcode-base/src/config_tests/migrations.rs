#[test]
fn migrate_legacy_swarm_spawn_mode_flips_visible_to_inline_once() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("JCODE_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir.path());

    let config_path = dir.path().join("config.toml");
    let original = "[display]\ncentered = true\n\n[agents]\nswarm_spawn_mode = \"visible\"\nswarm_max_concurrent_agents = 32\n";
    std::fs::write(&config_path, original).expect("write config");

    assert!(
        Config::migrate_legacy_swarm_spawn_mode_once(),
        "migration should rewrite a legacy visible spawn mode"
    );
    let migrated = std::fs::read_to_string(&config_path).expect("read config");
    assert!(
        migrated.contains("swarm_spawn_mode = \"inline\""),
        "spawn mode should be flipped to inline: {migrated}"
    );
    // The rest of the file is untouched.
    assert!(migrated.contains("centered = true"));
    assert!(migrated.contains("swarm_max_concurrent_agents = 32"));
    let parsed: Config = toml::from_str(&migrated).expect("migrated config parses");
    assert_eq!(parsed.agents.swarm_spawn_mode, SwarmSpawnMode::Inline);

    // Marker written: a later explicit "visible" survives future launches.
    std::fs::write(&config_path, "[agents]\nswarm_spawn_mode = \"visible\"\n")
        .expect("write config");
    assert!(
        !Config::migrate_legacy_swarm_spawn_mode_once(),
        "migration must run at most once"
    );
    let content = std::fs::read_to_string(&config_path).expect("read config");
    assert!(content.contains("swarm_spawn_mode = \"visible\""));

    restore_env_var("JCODE_HOME", prev_home);
}
#[test]
fn migrate_legacy_swarm_spawn_mode_noops_without_visible_value() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("JCODE_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir.path());

    // No config file at all: no migration, but the marker is written.
    assert!(!Config::migrate_legacy_swarm_spawn_mode_once());
    assert!(
        dir.path()
            .join("migrations")
            .join("swarm-spawn-mode-inline")
            .exists(),
        "marker should be written even when there is nothing to migrate"
    );

    // Explicit non-visible values are never rewritten (marker already set,
    // but check the matcher too with a fresh home).
    let dir2 = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir2.path());
    let config_path = dir2.path().join("config.toml");
    std::fs::write(&config_path, "[agents]\nswarm_spawn_mode = \"headless\"\n")
        .expect("write config");
    assert!(!Config::migrate_legacy_swarm_spawn_mode_once());
    let content = std::fs::read_to_string(&config_path).expect("read config");
    assert!(content.contains("swarm_spawn_mode = \"headless\""));

    restore_env_var("JCODE_HOME", prev_home);
}
#[test]
fn migrate_idle_animation_off_flips_true_to_false_once() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("JCODE_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir.path());

    let config_path = dir.path().join("config.toml");
    let original = "[display]\ncentered = true\nidle_animation = true\nanimation_fps = 60\n";
    std::fs::write(&config_path, original).expect("write config");

    assert!(
        Config::migrate_idle_animation_off_once(),
        "migration should rewrite an enabled idle animation"
    );
    let migrated = std::fs::read_to_string(&config_path).expect("read config");
    assert!(
        migrated.contains("idle_animation = false"),
        "idle animation should be flipped off: {migrated}"
    );
    // The rest of the file is untouched.
    assert!(migrated.contains("centered = true"));
    assert!(migrated.contains("animation_fps = 60"));
    let parsed: Config = toml::from_str(&migrated).expect("migrated config parses");
    assert!(!parsed.display.idle_animation);

    // Marker written: a later explicit re-enable survives future launches.
    std::fs::write(&config_path, "[display]\nidle_animation = true\n").expect("write config");
    assert!(
        !Config::migrate_idle_animation_off_once(),
        "migration must run at most once"
    );
    let content = std::fs::read_to_string(&config_path).expect("read config");
    assert!(content.contains("idle_animation = true"));

    restore_env_var("JCODE_HOME", prev_home);
}
#[test]
fn migrate_idle_animation_off_noops_without_enabled_value() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("JCODE_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir.path());

    // No config file at all: no migration, but the marker is written.
    assert!(!Config::migrate_idle_animation_off_once());
    assert!(
        dir.path()
            .join("migrations")
            .join("idle-animation-off")
            .exists(),
        "marker should be written even when there is nothing to migrate"
    );

    // Already-false values are never rewritten (fresh home to bypass marker).
    let dir2 = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir2.path());
    let config_path = dir2.path().join("config.toml");
    let original = "[display]\nidle_animation = false\n";
    std::fs::write(&config_path, original).expect("write config");
    assert!(!Config::migrate_idle_animation_off_once());
    let content = std::fs::read_to_string(&config_path).expect("read config");
    assert_eq!(content, original);

    restore_env_var("JCODE_HOME", prev_home);
}
#[test]
fn frozen_machine_written_sponsors_optout_is_repaired() {
    let raw = "[sponsors]\nenabled = false\nendpoint = \"https://api.jcode.sh/v1/discovery\"\n";
    let mut config: Config = toml::from_str(raw).expect("parse");
    assert!(!config.sponsors.enabled);
    config.repair_frozen_sponsors_optout(raw);
    assert!(
        config.sponsors.enabled,
        "a whole-struct config save must not permanently disable discovery"
    );
}
/// End-to-end: a real config file frozen by an old save must load with
/// discovery enabled, and the next save must drop the section entirely so the
/// freeze cannot recur.
#[test]
fn frozen_sponsors_optout_recovers_through_a_real_config_file() {
    let _guard = crate::storage::lock_test_env();
    let prev_home = std::env::var_os("JCODE_HOME");
    let dir = tempfile::TempDir::new().expect("tempdir");
    crate::env::set_var("JCODE_HOME", dir.path());
    Config::invalidate_cache();

    let path = Config::path().expect("config path");
    std::fs::create_dir_all(path.parent().expect("config parent")).expect("create config parent");
    std::fs::write(
        &path,
        "[display]\ncentered = false\n\n[sponsors]\nenabled = false\nendpoint = \"https://api.jcode.sh/v1/discovery\"\n",
    )
    .expect("write frozen config");

    let loaded = Config::load();
    assert!(
        loaded.sponsors.enabled,
        "loading a machine-frozen opt-out must restore the shipped default"
    );

    loaded.save().expect("save config");
    let rewritten = std::fs::read_to_string(&path).expect("read config");
    assert!(
        !rewritten.contains("[sponsors]"),
        "saving must not write the discovery section back: {rewritten}"
    );
    assert!(
        Config::load().sponsors.enabled,
        "discovery must stay enabled after a save/load round trip"
    );

    if let Some(prev) = prev_home {
        crate::env::set_var("JCODE_HOME", prev);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    Config::invalidate_cache();
}
#[test]
fn legacy_endpoint_optout_is_also_repaired() {
    let raw =
        "[sponsors]\nenabled = false\nendpoint = \"https://api.solosystems.dev/v1/discovery\"\n";
    let mut config: Config = toml::from_str(raw).expect("parse");
    config.repair_frozen_sponsors_optout(raw);
    assert!(config.sponsors.enabled);
}
#[test]
fn hand_written_sponsors_optout_is_respected() {
    for raw in [
        "[sponsors]\nenabled = false\n",
        "[sponsors]\nenabled = false\nendpoint = \"https://discovery.internal/v1\"\n",
    ] {
        let mut config: Config = toml::from_str(raw).expect("parse");
        config.repair_frozen_sponsors_optout(raw);
        assert!(
            !config.sponsors.enabled,
            "explicit user opt-out must survive: {raw}"
        );
    }
}
#[test]
fn default_sponsors_section_is_not_written_back() {
    let config = Config::default();
    let rendered = toml::to_string_pretty(&config).expect("serialize");
    assert!(
        !rendered.contains("[sponsors]"),
        "default discovery settings must not be baked into config.toml"
    );
}
