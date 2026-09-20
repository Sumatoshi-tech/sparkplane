//! Offline, typed namespace transition. Source databases and audit bytes remain untouched.
use crate::spark::{
    engine::EnginePolicy,
    wire::{InstanceDocument, ModelDocument},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, backup::Backup};
use std::{fs, os::unix::fs::OpenOptionsExt, path::Path, time::Duration};

/// Prepare an independent, integrity-checked snapshot. Never modifies the source.
/// Engine declarations must have been verified against the new signed release first.
pub fn stage_database(source: &Path, destination: &Path, engines: &[EnginePolicy]) -> Result<()> {
    ensure!(
        !destination.exists(),
        "migration destination already exists"
    );
    let original = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let version: u32 = original.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    ensure!(
        version == crate::spark::state::STATE_SCHEMA_VERSION,
        "unsupported migration source schema {version}"
    );
    ensure!(
        original.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))? == "ok",
        "source database is corrupt"
    );
    let active: i64 = original.query_row(
        "SELECT COUNT(*) FROM operations WHERE state NOT IN ('succeeded','failed','cancelled')",
        [],
        |row| row.get(0),
    )?;
    ensure!(active == 0, "drain active operations before migration");
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)?;
    let mut staged = Connection::open(destination)?;
    Backup::new(&original, &mut staged)?.run_to_completion(32, Duration::from_millis(10), None)?;
    ensure!(
        staged.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))? == "ok",
        "migration backup verification failed"
    );
    transform_state(&mut staged, engines)?;
    staged.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")?;
    drop(staged);
    fs::File::open(destination)?.sync_all()?;
    fs::File::open(
        destination
            .parent()
            .context("database destination has no parent")?,
    )?
    .sync_all()?;
    Ok(())
}

fn transform_state(connection: &mut Connection, engines: &[EnginePolicy]) -> Result<()> {
    let transaction = connection.transaction()?;
    let models = {
        let mut statement =
            transaction.prepare("SELECT id,metadata_json FROM models ORDER BY id")?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, json) in models {
        let mut model: ModelDocument = serde_json::from_str(&json)?;
        ensure!(
            model.schema == "sy.spark.model/v1",
            "unsupported legacy model schema"
        );
        ensure!(model.id == id, "model primary key and metadata disagree");
        model.schema = crate::spark::wire::MODEL_SCHEMA.into();
        if let Some(artifacts) = &mut model.artifacts {
            migrate_artifacts(artifacts)?;
        }
        if let Some(relative) = model
            .snapshot
            .strip_prefix("/var/lib/sy-spark/huggingface/")
        {
            model.snapshot = format!("/var/lib/sparkplane/huggingface/{relative}");
        }
        // Canonical model/checkpoint identities are content identities, not schema names.
        transaction.execute(
            "UPDATE models SET metadata_json=?2 WHERE id=?1",
            [&id, &serde_json::to_string(&model)?],
        )?;
    }
    let instances = {
        let mut statement =
            transaction.prepare("SELECT id,metadata_json FROM instances ORDER BY id")?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, json) in instances {
        let mut instance: InstanceDocument = serde_json::from_str(&json)?;
        ensure!(
            instance.schema == "sy.spark.instance/v2" && instance.id == id,
            "unsupported or inconsistent legacy instance"
        );
        ensure!(
            !instance.restart_suppressed && instance.quarantine.is_none(),
            "resolve suppressed/quarantined instance {} before migration",
            instance.name
        );
        instance.schema = crate::spark::wire::INSTANCE_SCHEMA.into();
        migrate_artifacts(&mut instance.artifacts)?;
        let engine = engines
            .iter()
            .find(|engine| engine.config().id == instance.engine_id)
            .context("signed release does not contain the instance engine")?;
        let profile = engine
            .profile_for(None, &instance.artifacts)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            profile.context_window == instance.context_window,
            "migration must preserve instance context window"
        );
        instance.engine_fingerprint = engine.fingerprint().into();
        instance.artifact_fingerprint =
            crate::spark::wire::artifact_fingerprint(&instance.artifacts)
                .map_err(anyhow::Error::msg)?;
        instance.observed = crate::spark::wire::InstanceObservedState::Absent;
        instance.healthy = false;
        instance.endpoint = None;
        instance.started_at = None;
        transaction.execute(
            "UPDATE instances SET observed_state='absent',metadata_json=?2 WHERE id=?1",
            [&id, &serde_json::to_string(&instance)?],
        )?;
    }
    // These are live coordination data, not audit history. Preflight requires no active operation.
    transaction.execute("DELETE FROM transition_leases", [])?;
    transaction.execute("UPDATE instance_resources SET phase='failed',current_memory_bytes=0,previous_memory_bytes=0", [])?;
    transaction.execute("UPDATE token_metadata SET name=replace(name,'sy-launch@','sparkplane-launch@') WHERE name LIKE 'sy-launch@%'", [])?;
    // audit, operations, events, emergency evidence and verifier bytes remain byte-for-byte intact.
    transaction.commit()?;
    Ok(())
}

fn migrate_artifacts(artifacts: &mut crate::spark::wire::ModelArtifactsDocument) -> Result<()> {
    ensure!(
        artifacts.schema == "sy.spark.model-artifacts/v2",
        "unsupported legacy artifact schema"
    );
    artifacts.schema = "sparkplane.model-artifacts/v2".into();
    Ok(())
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub schema: String,
    pub host_identity: String,
    pub release_sha256: String,
    pub new_public_key: String,
    pub expires_at_unix_seconds: u64,
}

pub fn verify_transition(
    bytes: &[u8],
    signature: &str,
    installed_key: &str,
    host: &str,
    release: &str,
    now: u64,
) -> Result<Transition> {
    ensure!(
        bytes.len() <= 16 * 1024,
        "transition manifest exceeds size limit"
    );
    let authority = minisign_verify::PublicKey::from_base64(installed_key.trim())?;
    authority.verify(
        bytes,
        &minisign_verify::Signature::decode(signature)?,
        false,
    )?;
    let transition: Transition = serde_json::from_slice(bytes)?;
    ensure!(
        transition.schema == "sparkplane.trust-transition/v1",
        "unsupported trust transition schema"
    );
    ensure!(
        transition.host_identity == host
            && host.len() == 64
            && host.bytes().all(|b| b.is_ascii_hexdigit()),
        "trust transition is for a different host"
    );
    ensure!(
        transition.release_sha256 == release
            && release.len() == 64
            && release.bytes().all(|b| b.is_ascii_hexdigit()),
        "trust transition is for a different release"
    );
    ensure!(
        now < transition.expires_at_unix_seconds,
        "trust transition expired"
    );
    minisign_verify::PublicKey::from_base64(&transition.new_public_key)?;
    Ok(transition)
}
