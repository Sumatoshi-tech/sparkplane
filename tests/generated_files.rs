#[test]
fn edited_generated_files_are_preserved_and_other_files_are_not_changed() {
    let root = tempfile::tempdir().unwrap();
    sparkplane::generated_files::publish(root.path(), b"profile", b"catalog").unwrap();
    let profile = root.path().join("sparkplane-launch.config.toml");
    std::fs::write(&profile, b"user edits").unwrap();
    assert!(sparkplane::generated_files::publish(root.path(), b"next", b"next catalog").is_err());
    assert_eq!(std::fs::read(profile).unwrap(), b"user edits");
    assert_eq!(
        std::fs::read(root.path().join("sparkplane-launch-models.json")).unwrap(),
        b"catalog"
    );
}

#[test]
fn removal_checks_both_files_before_deleting_either() {
    let root = tempfile::tempdir().unwrap();
    sparkplane::generated_files::publish(root.path(), b"profile", b"catalog").unwrap();
    std::fs::write(root.path().join("sparkplane-launch-models.json"), b"edited").unwrap();
    assert!(sparkplane::generated_files::remove(root.path()).is_err());
    assert!(root.path().join("sparkplane-launch.config.toml").exists());
}

#[test]
fn unowned_marker_and_symlink_do_not_grant_permission_to_overwrite() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("sparkplane-launch.config.toml");
    std::fs::write(&profile, b"# owned-by: sparkplane launch\nuser changes").unwrap();
    assert!(sparkplane::generated_files::publish(root.path(), b"next", b"catalog").is_err());
    std::fs::rename(&profile, root.path().join("user-original")).unwrap();
    std::os::unix::fs::symlink("user-original", &profile).unwrap();
    assert!(sparkplane::generated_files::publish(root.path(), b"next", b"catalog").is_err());
}

#[test]
fn codex_profile_settings_do_not_block_republication_or_restore() {
    let root = tempfile::tempdir().unwrap();
    let profile = b"# owned-by: sparkplane launch\nmodel = \"m\"\nmodel_provider = \"p\"\nweb_search = \"disabled\"\n\n[model_providers.p]\nname = \"n\"\nbase_url = \"https://example/v1\"\nenv_key = \"SPARKPLANE_INFERENCE_TOKEN\"\nwire_api = \"responses\"\nsupports_standalone_web_search = false\nsupports_websockets = false\n";
    sparkplane::generated_files::publish(root.path(), profile, b"catalog").unwrap();
    let edited = std::str::from_utf8(profile).unwrap().replace(
        "web_search = \"disabled\"\n",
        "web_search = \"disabled\"\napprovals_reviewer = \"user\"\n",
    );
    let path = root.path().join("sparkplane-launch.config.toml");
    std::fs::write(&path, &edited).unwrap();
    let next = b"# owned-by: sparkplane launch\nmodel = \"next\"\nmodel_provider = \"p\"\nweb_search = \"disabled\"\n\n[model_providers.p]\nname = \"n\"\nbase_url = \"https://example/v1\"\nenv_key = \"SPARKPLANE_INFERENCE_TOKEN\"\nwire_api = \"responses\"\nsupports_standalone_web_search = false\nsupports_websockets = false\n";
    sparkplane::generated_files::publish(root.path(), next, b"next catalog").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), next);
    std::fs::write(
        &path,
        std::str::from_utf8(next).unwrap().replace(
            "web_search = \"disabled\"\n",
            "web_search = \"disabled\"\napprovals_reviewer = \"user\"\n",
        ),
    )
    .unwrap();
    sparkplane::generated_files::remove(root.path()).unwrap();
    assert!(!path.exists());
}

#[test]
fn a_changed_managed_line_names_the_file_and_leaves_it_in_place() {
    let root = tempfile::tempdir().unwrap();
    let profile = b"model = \"m\"\nweb_search = \"disabled\"\n";
    sparkplane::generated_files::publish(root.path(), profile, b"catalog").unwrap();
    let path = root.path().join("sparkplane-launch.config.toml");
    std::fs::write(&path, b"model = \"edited\"\nweb_search = \"disabled\"\n").unwrap();
    let error =
        sparkplane::generated_files::publish(root.path(), b"model = \"next\"\n", b"catalog")
            .unwrap_err()
            .to_string();
    assert!(error.contains(&path.display().to_string()), "{error}");
    assert!(error.contains("Move it aside and retry"), "{error}");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"model = \"edited\"\nweb_search = \"disabled\"\n"
    );
}

#[test]
fn an_interrupted_pair_update_can_be_completed_without_losing_ownership() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    sparkplane::generated_files::publish(root.path(), b"profile", b"catalog").unwrap();
    let path = root.path().join("sparkplane-launch.ownership.json");
    let mut pending: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    pending["files"]["sparkplane-launch.config.toml"]
        .as_array_mut()
        .unwrap()
        .push(format!("{:x}", Sha256::digest(b"next")).into());
    std::fs::write(path, serde_json::to_vec(&pending).unwrap()).unwrap();
    std::fs::write(root.path().join("sparkplane-launch.config.toml"), b"next").unwrap();
    sparkplane::generated_files::publish(root.path(), b"final", b"final catalog").unwrap();
    sparkplane::generated_files::remove(root.path()).unwrap();
    assert!(!root.path().join("sparkplane-launch.config.toml").exists());
}

#[test]
fn resume_providers_survive_model_changes_and_profile_removal() {
    let root = tempfile::tempdir().unwrap();
    let original = "# User preferences\nmodel = 'gpt-6.1-sol'\n[model_providers.other]\nname = 'Other'\nbase_url = 'https://other.example/v1'\nwire_api = 'responses'\n";
    std::fs::write(root.path().join("config.toml"), original).unwrap();
    let first = b"model='qwen'\nmodel_provider='sparkplane_qwen'\n[model_providers.sparkplane_qwen]\nname='Sparkplane Qwen'\nbase_url='https://spark.example/openai/qwen/v1'\nenv_key='SPARKPLANE_INFERENCE_TOKEN'\nwire_api='responses'\n";
    let second = b"model='other'\nmodel_provider='sparkplane_other'\n[model_providers.sparkplane_other]\nname='Sparkplane Other'\nbase_url='https://spark.example/openai/other/v1'\nenv_key='SPARKPLANE_INFERENCE_TOKEN'\nwire_api='responses'\n";
    for profile in [first.as_slice(), second.as_slice()] {
        sparkplane::generated_files::publish(root.path(), profile, b"catalog").unwrap();
        sparkplane::generated_files::publish_provider(root.path(), profile).unwrap();
    }
    sparkplane::generated_files::remove(root.path()).unwrap();
    let text = std::fs::read_to_string(root.path().join("config.toml")).unwrap();
    let config: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(config["model"].as_str(), Some("gpt-6.1-sol"));
    assert!(config.get("model_provider").is_none());
    assert!(text.starts_with("# User preferences\nmodel = 'gpt-6.1-sol'\n"));
    for name in ["sparkplane_qwen", "sparkplane_other", "other"] {
        assert!(config["model_providers"].get(name).is_some(), "{name}");
    }
    assert!(!root.path().join("sparkplane-launch.config.toml").exists());
    // Missing definitions are repaired without replacing user preferences.
    std::fs::write(root.path().join("config.toml"), original).unwrap();
    sparkplane::generated_files::publish_provider(root.path(), first).unwrap();
    let text = std::fs::read_to_string(root.path().join("config.toml")).unwrap();
    assert!(text.contains("sparkplane_qwen"));
}

#[test]
fn resume_provider_registration_preserves_edits_and_rejects_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let profile = b"[model_providers.sparkplane_qwen]\nname='Sparkplane Qwen'\nbase_url='https://spark.example/v1'\nwire_api='responses'\n";
    sparkplane::generated_files::publish_provider(root.path(), profile).unwrap();
    let path = root.path().join("config.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let edited = text.replace("https://spark.example/v1", "https://custom.example/v1");
    std::fs::write(&path, &edited).unwrap();
    assert!(sparkplane::generated_files::publish_provider(root.path(), profile).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    std::fs::rename(&path, root.path().join("user-original")).unwrap();
    std::os::unix::fs::symlink("user-original", &path).unwrap();
    assert!(sparkplane::generated_files::publish_provider(root.path(), profile).is_err());
}
