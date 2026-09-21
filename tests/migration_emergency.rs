#![cfg(feature = "appliance")]
use sparkplane::migration::emergency;
use std::{fs, os::unix::fs::PermissionsExt};

fn fixture() -> (tempfile::TempDir, Vec<u8>) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("executor")).unwrap();
    let record = serde_json::json!({"schema":"sy.spark.emergency-record/v1",
        "event_id":"01M303PWWXR02FK2H7XN15NV7B","occurred_at_unix_ms":123,
        "decision":{"schema":"sy.spark.emergency-decision/v1","instance_id":"fixture",
        "generation":8,"cause":"memory floor","mem_available_bytes":7,
        "memory_full_psi_avg10_percent":0.0}});
    let bytes = format!("{record}\n").into_bytes();
    let path = root.path().join("executor/emergency.jsonl");
    fs::write(&path, &bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    let db = rusqlite::Connection::open(root.path().join("state.sqlite3")).unwrap();
    db.execute_batch(
        "CREATE TABLE emergency_records(event_id TEXT PRIMARY KEY,evidence_json TEXT NOT NULL)",
    )
    .unwrap();
    db.execute(
        "INSERT INTO emergency_records VALUES(?1,?2)",
        [record["event_id"].as_str().unwrap(), &record.to_string()],
    )
    .unwrap();
    (root, bytes)
}

#[test]
fn emergency_import_changes_only_typed_namespace_and_preserves_source() {
    let (root, original) = fixture();
    let candidate = emergency::validate(root.path()).unwrap().unwrap();
    let mut expected: serde_json::Value = serde_json::from_slice(&original).unwrap();
    expected["schema"] = "sparkplane.emergency-record/v1".into();
    expected["decision"]["schema"] = "sparkplane.emergency-decision/v1".into();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&candidate).unwrap(),
        expected
    );
    assert_eq!(
        fs::read(root.path().join("executor/emergency.jsonl")).unwrap(),
        original
    );
}

#[test]
fn emergency_cutover_and_recovery_preserve_original_bytes() {
    let (root, original) = fixture();
    let work = tempfile::tempdir().unwrap();
    let journal = emergency::Journal::prepare(root.path(), work.path()).unwrap();
    journal.activate(root.path(), work.path()).unwrap();
    journal.activate(root.path(), work.path()).unwrap();
    assert!(
        fs::read_to_string(root.path().join("executor/emergency.jsonl"))
            .unwrap()
            .contains("sparkplane.emergency-record/v1")
    );
    journal.restore(root.path(), work.path()).unwrap();
    journal.restore(root.path(), work.path()).unwrap();
    assert_eq!(
        fs::read(root.path().join("executor/emergency.jsonl")).unwrap(),
        original
    );
}

#[test]
fn emergency_rejects_unimported_or_changed_evidence_and_unknown_schema() {
    for mutation in [
        "DELETE FROM emergency_records",
        "UPDATE emergency_records SET evidence_json='{}'",
    ] {
        let (root, _) = fixture();
        rusqlite::Connection::open(root.path().join("state.sqlite3"))
            .unwrap()
            .execute_batch(mutation)
            .unwrap();
        assert!(emergency::validate(root.path()).is_err());
    }
    let (root, original) = fixture();
    fs::write(
        root.path().join("executor/emergency.jsonl"),
        String::from_utf8(original)
            .unwrap()
            .replace("sy.spark.emergency-record/v1", "unknown/v1"),
    )
    .unwrap();
    assert!(emergency::validate(root.path()).is_err());
}

#[test]
fn every_partial_emergency_exchange_recovers_twice_without_discarding_new_records() {
    for cutoff in 0..=2 {
        let (root, original) = fixture();
        let work = tempfile::tempdir().unwrap();
        let journal = emergency::Journal::prepare(root.path(), work.path()).unwrap();
        let active = root.path().join("executor/emergency.jsonl");
        if cutoff >= 1 {
            fs::rename(&active, work.path().join("original-emergency.jsonl")).unwrap();
        }
        if cutoff == 2 {
            fs::rename(work.path().join("candidate-emergency.jsonl"), &active).unwrap();
            fs::write(&active, b"failed generation evidence").unwrap();
        }
        journal.restore(root.path(), work.path()).unwrap();
        journal.restore(root.path(), work.path()).unwrap();
        assert_eq!(fs::read(active).unwrap(), original);
        if cutoff == 2 {
            assert_eq!(
                fs::read(work.path().join("failed-emergency.jsonl")).unwrap(),
                b"failed generation evidence"
            );
        }
    }
}

#[test]
fn emergency_absence_and_symlink_conflicts_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let journal = emergency::Journal::prepare(root.path(), work.path()).unwrap();
    journal.activate(root.path(), work.path()).unwrap();
    fs::create_dir(root.path().join("executor")).unwrap();
    fs::write(
        root.path().join("executor/emergency.jsonl"),
        b"new evidence",
    )
    .unwrap();
    journal.restore(root.path(), work.path()).unwrap();
    journal.restore(root.path(), work.path()).unwrap();
    assert_eq!(
        fs::read(work.path().join("failed-emergency.jsonl")).unwrap(),
        b"new evidence"
    );
    std::os::unix::fs::symlink(
        work.path().join("failed-emergency.jsonl"),
        root.path().join("executor/emergency.jsonl"),
    )
    .unwrap();
    assert!(emergency::validate(root.path()).is_err());
}
