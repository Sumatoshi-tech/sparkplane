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
