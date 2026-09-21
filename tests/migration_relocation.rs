#![cfg(feature = "appliance")]
use sparkplane::migration::relocation::{Identity, relocate};

#[test]
fn repeated_relocation_preserves_cache_bytes_and_inode() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("legacy");
    let new = root.path().join("current");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("weights"), b"unchanged").unwrap();
    let identity = Identity::read(&old).unwrap();
    relocate(&old, &new, &identity).unwrap();
    relocate(&old, &new, &identity).unwrap();
    assert_eq!(Identity::read(&new).unwrap(), identity);
    assert_eq!(std::fs::read(new.join("weights")).unwrap(), b"unchanged");
}

#[test]
fn destination_conflict_and_replaced_source_leave_both_trees_untouched() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("legacy");
    let new = root.path().join("current");
    std::fs::create_dir(&old).unwrap();
    std::fs::create_dir(&new).unwrap();
    let identity = Identity::read(&old).unwrap();
    assert!(relocate(&old, &new, &identity).is_err());
    std::fs::rename(&old, root.path().join("replaced")).unwrap();
    std::fs::create_dir(&old).unwrap();
    assert!(relocate(&old, &new, &identity).is_err());
    assert!(old.is_dir() && new.is_dir());
}

#[test]
fn journaled_file_move_is_idempotent_without_overwriting_a_conflict() {
    use sparkplane::migration::relocation::{FileIdentity, move_file};
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("source");
    let new = root.path().join("destination");
    std::fs::write(&old, b"database fixture").unwrap();
    let identity = FileIdentity::read(&old).unwrap();
    move_file(&old, &new, &identity).unwrap();
    move_file(&old, &new, &identity).unwrap();
    assert_eq!(FileIdentity::read(&new).unwrap(), identity);
    std::fs::write(&old, b"unrelated").unwrap();
    assert!(move_file(&old, &new, &identity).is_err());
}

#[test]
fn changed_file_permissions_cannot_be_published() {
    use sparkplane::migration::relocation::{FileIdentity, move_file};
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::write(&source, b"same bytes").unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    let expected = FileIdentity::read(&source).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(move_file(&source, &destination, &expected).is_err());
    assert!(source.exists() && !destination.exists());
}
