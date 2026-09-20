use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

#[test]
fn dry_run_does_not_copy_credentials_or_create_destination() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("sy");
    let destination = root.path().join("sparkplane");
    fs::create_dir_all(source.join("credentials/spark")).unwrap();
    fs::write(source.join("spark.toml"), "[hosts]\n").unwrap();
    fs::write(source.join("credentials/spark/test"), "secret").unwrap();
    let report = sparkplane::client_migration::migrate(&source, &destination, true).unwrap();
    assert_eq!(report.files, 2);
    assert!(!destination.exists());
    assert_eq!(
        fs::read_to_string(source.join("credentials/spark/test")).unwrap(),
        "secret"
    );
}

#[test]
fn migration_preserves_private_credential_bytes_and_unrelated_settings() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("sy");
    let destination = root.path().join("sparkplane");
    fs::create_dir_all(source.join("credentials/spark")).unwrap();
    fs::write(source.join("spark.toml"), "[hosts]\n").unwrap();
    fs::write(source.join("credentials/spark/test"), "secret\n").unwrap();
    fs::write(source.join("desktop.toml"), "unrelated=true\n").unwrap();
    sparkplane::client_migration::migrate(&source, &destination, false).unwrap();
    let credential = destination.join("credentials/spark/test");
    assert_eq!(fs::read(&credential).unwrap(), b"secret\n");
    assert_eq!(
        fs::metadata(credential).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!destination.join("desktop.toml").exists());
    assert!(source.join("desktop.toml").exists());
    assert!(sparkplane::client_migration::migrate(&source, &destination, false).is_err());
}

#[test]
fn symlinked_credentials_are_rejected_before_any_copy() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("sy");
    let destination = root.path().join("sparkplane");
    fs::create_dir_all(source.join("credentials/spark")).unwrap();
    fs::write(source.join("spark.toml"), "[hosts]\n").unwrap();
    symlink("/etc/passwd", source.join("credentials/spark/test")).unwrap();
    assert!(sparkplane::client_migration::migrate(&source, &destination, false).is_err());
    assert!(!destination.exists());
}

#[test]
fn symlinked_credential_ancestor_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("sy");
    fs::create_dir(&source).unwrap();
    fs::create_dir_all(root.path().join("external/spark")).unwrap();
    fs::write(source.join("spark.toml"), "[hosts]\n").unwrap();
    symlink(root.path().join("external"), source.join("credentials")).unwrap();
    assert!(
        sparkplane::client_migration::migrate(&source, &root.path().join("new"), true).is_err()
    );
}
