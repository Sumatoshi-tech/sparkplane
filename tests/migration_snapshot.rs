#![cfg(feature = "appliance")]
use sparkplane::migration::snapshot_path;

#[test]
fn snapshot_must_match_the_exact_repository_and_immutable_commit() {
    let commit = "a".repeat(40);
    let relative = format!("models--owner--model/snapshots/{commit}");
    assert_eq!(
        snapshot_path(&relative, "owner/model", &commit).unwrap(),
        relative
    );
    for path in [
        "../../outside",
        "/var/lib/sy-spark/huggingface/../outside",
        "models--other--model/snapshots/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert!(snapshot_path(path, "owner/model", &commit).is_err());
    }
}
