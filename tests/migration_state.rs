#![cfg(feature = "appliance")]
use sparkplane::migration::stage_database;

#[test]
fn unsupported_database_version_is_rejected_without_a_destination() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("old.sqlite3");
    let destination = root.path().join("new.sqlite3");
    rusqlite::Connection::open(&source)
        .unwrap()
        .execute_batch("PRAGMA user_version=999;")
        .unwrap();
    assert!(stage_database(&source, &destination, &[]).is_err());
    assert!(!destination.exists());
}

#[test]
fn failed_metadata_conversion_does_not_publish_a_partial_database() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("old.sqlite3");
    let destination = root.path().join("new.sqlite3");
    rusqlite::Connection::open(&source).unwrap().execute_batch(
        "PRAGMA user_version=6; CREATE TABLE operations(state TEXT); CREATE TABLE models(id TEXT,metadata_json TEXT); INSERT INTO models VALUES('bad','invalid json');"
    ).unwrap();
    assert!(stage_database(&source, &destination, &[]).is_err());
    assert!(!destination.exists());
}

#[test]
fn wal_snapshot_preserves_identity_credentials_and_historical_audit_bytes() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("old.sqlite3");
    let destination = root.path().join("new.sqlite3");
    let _actor = sparkplane::spark::state::DbActor::open(
        &source,
        root.path().join("backups"),
        8,
        2,
        secrecy::SecretString::from("fixture-pepper"),
    )
    .unwrap();
    let original = rusqlite::Connection::open(&source).unwrap();
    original
        .execute_batch("PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    let model = serde_json::json!({
        "schema":"sy.spark.model/v1", "id":"model-id", "canonical":"huggingface:owner/model@immutable#sha256:unchanged",
        "repository":"owner/model", "commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "snapshot":"/var/lib/sy-spark/huggingface/models--owner--model/snapshots/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "artifacts":null, "logical_bytes":8, "unique_bytes":8, "aliases":["my-model"], "active_instances":[],
        "transport":"fixture", "verified_at":"2026-09-21T00:00:00Z", "gated":false, "license":"MIT"
    }).to_string();
    original
        .execute(
            "INSERT INTO models VALUES('model-id','owner/model',?1,?2)",
            [&"a".repeat(40), &model],
        )
        .unwrap();
    original
        .execute("INSERT INTO aliases VALUES('my-model','model-id')", [])
        .unwrap();
    const AUDIT: &str = "{ \"schema\": \"sy.spark.historical/v1\", \"unchanged\": true }";
    original.execute("INSERT INTO audit(occurred_at,actor_token_id,action,outcome,metadata_json) VALUES('now','admin','fixture','succeeded',?1)", [AUDIT]).unwrap();
    original.execute("INSERT INTO token_metadata(id,name,verifier,scopes_json,allowed_cidrs_json,max_concurrent_inference,created_at) VALUES('token','sy-launch@host',x'010203','[]','[]',1,'now')", []).unwrap();
    assert!(source.with_extension("sqlite3-wal").exists());
    stage_database(&source, &destination, &[]).unwrap();
    let staged = rusqlite::Connection::open(&destination).unwrap();
    let migrated: String = staged
        .query_row("SELECT metadata_json FROM models", [], |row| row.get(0))
        .unwrap();
    let migrated: serde_json::Value = serde_json::from_str(&migrated).unwrap();
    assert_eq!(migrated["schema"], "sparkplane.model/v1");
    assert_eq!(
        migrated["canonical"],
        "huggingface:owner/model@immutable#sha256:unchanged"
    );
    assert!(
        migrated["snapshot"]
            .as_str()
            .unwrap()
            .starts_with("/var/lib/sparkplane/")
    );
    assert_eq!(
        staged
            .query_row("SELECT metadata_json FROM audit", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        AUDIT
    );
    assert_eq!(
        staged
            .query_row("SELECT hex(verifier) FROM token_metadata", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
        "010203"
    );
    assert_eq!(
        original
            .query_row("SELECT metadata_json FROM models", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        model
    );
}
