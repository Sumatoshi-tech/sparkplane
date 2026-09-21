#![cfg(feature = "appliance")]
use sparkplane::migration::storage::Storage;

#[test]
fn filesystem_cutover_and_recovery_preserve_original_database_and_weights() {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    let data = root.path().join("var/lib/sy-spark");
    std::fs::create_dir_all(data.join("huggingface")).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(data.join("huggingface/weights"), b"unchanged").unwrap();
    let _actor = sparkplane::spark::state::DbActor::open(
        data.join("state.sqlite3"),
        data.join("backups"),
        8,
        2,
        secrecy::SecretString::from("fixture"),
    )
    .unwrap();
    let original = std::fs::read(data.join("state.sqlite3")).unwrap();
    let emergency = r#"{"schema":"sy.spark.emergency-record/v1","event_id":"01M303PWWXR02FK2H7XN15NV7B","occurred_at_unix_ms":123,"decision":{"schema":"sy.spark.emergency-decision/v1","instance_id":"fixture","generation":8,"cause":"floor","mem_available_bytes":7,"memory_full_psi_avg10_percent":0.0}}"#;
    std::fs::create_dir(data.join("executor")).unwrap();
    let journal_path = data.join("executor/emergency.jsonl");
    std::fs::write(&journal_path, emergency).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&journal_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    rusqlite::Connection::open(data.join("state.sqlite3")).unwrap().execute("INSERT INTO emergency_records VALUES('01M303PWWXR02FK2H7XN15NV7B','fixture',8,'floor',?1,'fixture')", [emergency]).unwrap();
    let storage = Storage::prepare(root.path(), &work, &[], &[]).unwrap();
    storage.activate(root.path(), &work).unwrap();
    storage.activate(root.path(), &work).unwrap();
    assert!(
        std::fs::read_to_string(
            root.path()
                .join("var/lib/sparkplane/executor/emergency.jsonl")
        )
        .unwrap()
        .contains("sparkplane.emergency-record/v1")
    );
    let current = root.path().join("var/lib/sparkplane");
    let replay = sparkplane::spark::resources::read_emergency_records(
        &current.join("executor/emergency.jsonl"),
    )
    .unwrap();
    let actor = sparkplane::spark::state::DbActor::open(
        current.join("state.sqlite3"),
        current.join("backups"),
        8,
        2,
        secrecy::SecretString::from("fixture"),
    )
    .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert!(
        !runtime
            .block_on(actor.import_emergency(replay[0].clone()))
            .unwrap()
    );
    drop(actor);
    assert_eq!(
        std::fs::read(root.path().join("var/lib/sparkplane/huggingface/weights")).unwrap(),
        b"unchanged"
    );
    storage.restore(root.path(), &work).unwrap();
    storage.restore(root.path(), &work).unwrap();
    assert_eq!(std::fs::read_to_string(journal_path).unwrap(), emergency);
    assert!(
        std::fs::read(data.join("state.sqlite3")).unwrap() == original,
        "original database bytes must survive recovery"
    );
    assert!(!root.path().join("var/lib/sparkplane").exists());
}

#[test]
fn every_partial_database_and_cache_rename_can_be_recovered_twice() {
    use std::fs;
    for cutoff in 0..=7 {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("var/lib/sy-spark");
        let destination = root.path().join("var/lib/sparkplane");
        let work = root.path().join("work");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir(&work).unwrap();
        let _actor = sparkplane::spark::state::DbActor::open(
            data.join("state.sqlite3"),
            data.join("backups"),
            8,
            2,
            secrecy::SecretString::from("fixture"),
        )
        .unwrap();
        let keys = [
            (
                format!("sha256-{}", "a".repeat(64)),
                format!("sha256-{}", "b".repeat(64)),
            ),
            (
                format!("sha256-{}", "c".repeat(64)),
                format!("sha256-{}", "d".repeat(64)),
            ),
        ];
        for (old, _) in &keys {
            fs::create_dir_all(data.join("compile-cache").join(old)).unwrap();
            fs::write(
                data.join("compile-cache").join(old).join("compiled"),
                b"same cache",
            )
            .unwrap();
        }
        let storage = Storage::prepare(root.path(), &work, &[], &keys).unwrap();
        let recorded = serde_json::to_value(&storage).unwrap();
        let originals = recorded["original"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let name = row[0].as_str().unwrap().to_owned();
                let bytes = fs::read(data.join(&name)).unwrap();
                (name, bytes)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            originals.len(),
            3,
            "fixture must exercise the WAL and shared memory files"
        );
        let mut moves = originals
            .iter()
            .map(|(name, _)| (data.join(name), work.join("original-db").join(name)))
            .collect::<Vec<_>>();
        moves.push((work.join("candidate.sqlite3"), data.join("state.sqlite3")));
        moves.push((data.clone(), destination.clone()));
        moves.extend(keys.iter().map(|(old, new)| {
            (
                destination.join("compile-cache").join(old),
                destination.join("compile-cache").join(new),
            )
        }));
        for (from, to) in moves.iter().take(cutoff) {
            fs::rename(from, to).unwrap();
        }
        storage.restore(root.path(), &work).unwrap();
        storage.restore(root.path(), &work).unwrap();
        assert!(!destination.exists());
        for (name, bytes) in originals {
            assert!(fs::read(data.join(name)).unwrap() == bytes);
        }
        for (old, _) in &keys {
            assert_eq!(
                fs::read(data.join("compile-cache").join(old).join("compiled")).unwrap(),
                b"same cache"
            );
        }
    }
}
