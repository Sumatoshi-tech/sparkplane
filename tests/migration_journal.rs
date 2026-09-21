#![cfg(feature = "appliance")]
use sparkplane::migration::journal::{Journal, Step};

#[test]
fn replacement_authority_is_only_valid_after_rollback_restores_state() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    assert!(!journal.restored_checkpoint());
    for step in [
        Step::FenceTraffic,
        Step::Drain,
        Step::StopLegacyServices,
        Step::Snapshot,
    ] {
        journal.begin(step).unwrap();
        journal.finish(step).unwrap();
    }
    assert!(!journal.restored_checkpoint());
    assert!(
        journal
            .recover(|step| {
                anyhow::ensure!(step != Step::StopLegacyServices, "wait for health");
                Ok(())
            })
            .is_err()
    );
    assert!(journal.restored_checkpoint());
}

#[test]
fn interrupted_action_remains_pending_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    {
        let mut journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
        journal.begin(Step::FenceTraffic).unwrap();
    }
    let journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    assert_eq!(journal.pending(), Some(Step::FenceTraffic));
}

#[test]
fn migration_cannot_skip_the_traffic_fence() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    assert!(journal.begin(Step::Relocate).is_err());
}

#[test]
fn completed_action_survives_reopen_and_cannot_be_repeated() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    journal.begin(Step::FenceTraffic).unwrap();
    journal.finish(Step::FenceTraffic).unwrap();
    drop(journal);
    let mut journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    assert!(journal.begin(Step::FenceTraffic).is_err());
    journal.begin(Step::Drain).unwrap();
}

#[test]
fn rollback_retries_the_interrupted_undo_before_earlier_actions() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    journal.begin(Step::FenceTraffic).unwrap();
    journal.finish(Step::FenceTraffic).unwrap();
    journal.begin(Step::Drain).unwrap();
    assert!(journal.recover(|_| anyhow::bail!("interrupted")).is_err());
    drop(journal);
    let mut journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    let mut undone = vec![];
    journal
        .recover(|step| {
            undone.push(step);
            Ok(())
        })
        .unwrap();
    assert_eq!(undone, [Step::Drain, Step::FenceTraffic]);
}

#[test]
fn committed_migration_refuses_stale_snapshot_recovery() {
    let root = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    assert!(journal.commit().is_err());
    for step in [
        Step::FenceTraffic,
        Step::Drain,
        Step::StopLegacyServices,
        Step::Snapshot,
        Step::StopLegacyEngine,
        Step::Relocate,
        Step::Activate,
        Step::Qualify,
    ] {
        journal.begin(step).unwrap();
        journal.finish(step).unwrap();
    }
    journal.commit().unwrap();
    drop(journal);
    let mut journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    assert!(
        journal
            .recover(|_| panic!("must not restore after commit"))
            .is_err()
    );
}

#[test]
fn corrupted_action_history_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    drop(journal);
    let path = root.path().join("migration.json");
    let mut state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    state["completed"] = serde_json::json!(["relocate"]);
    std::fs::write(path, serde_json::to_vec(&state).unwrap()).unwrap();
    assert!(Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).is_err());
}

#[test]
fn migration_journal_rejects_a_shared_writable_directory() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).is_err());
}

#[test]
fn concurrent_runner_and_different_release_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let journal = Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).unwrap();
    assert!(Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).is_err());
    drop(journal);
    assert!(Journal::open(root.path(), &"a".repeat(64), &"c".repeat(64)).is_err());
}

#[test]
fn dangling_journal_link_is_not_replaced_with_a_fresh_history() {
    let root = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("missing", root.path().join("migration.json")).unwrap();
    assert!(Journal::open(root.path(), &"a".repeat(64), &"b".repeat(64)).is_err());
}
