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
