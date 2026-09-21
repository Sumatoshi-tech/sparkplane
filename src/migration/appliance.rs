//! Privileged cutover integration. The CLI fixes every host path and operation.
use super::{
    container::{LegacyContainer, Namespace, Network},
    publication::Publication,
    release::Release,
};
use crate::spark::{
    engine::EnginePolicy,
    wire::{InstanceDesiredState, InstanceDocument, ModelDocument},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    net::SocketAddr,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub const BASE: &str = "/var/lib/sparkplane-migration";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Active {
    pub before: InstanceDocument,
    pub container: LegacyContainer,
    pub current_engine_fingerprint: String,
    pub current_artifact_fingerprint: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: String,
    pub host: String,
    pub release: String,
    pub executable: String,
    pub uid: u32,
    pub gid: u32,
    pub listen: SocketAddr,
    pub active: Vec<Active>,
    pub cache_keys: Vec<(String, String)>,
    pub engines: BTreeMap<String, String>,
    pub agent_sha256: String,
    pub executor_sha256: String,
    pub inventory_sha256: String,
    pub source_engines: BTreeMap<String, String>,
    pub model_catalog_sha256: String,
    pub legacy_network: Option<Network>,
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn text(path: &Path) -> Result<String> {
    Ok(String::from_utf8(super::release::read(path, 1024 * 1024)?)?)
}

pub fn host_identity(root: &Path) -> Result<String> {
    let machine = text(&root.join("etc/machine-id"))?;
    let machine = machine.trim();
    ensure!(
        machine.len() == 32 && machine.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid machine identity"
    );
    Ok(digest(machine.as_bytes()))
}

pub fn account(root: &Path, name: &str, group: bool) -> Result<Option<u32>> {
    let contents = text(&root.join(if group { "etc/group" } else { "etc/passwd" }))?;
    let mut found = None;
    for line in contents.lines() {
        let fields: Vec<_> = line.split(':').collect();
        if fields.first() == Some(&name) {
            ensure!(found.is_none(), "duplicate service identity");
            found = Some(fields.get(2).context("invalid service account")?.parse()?);
        }
    }
    Ok(found)
}

pub fn available_disk(root: &Path) -> Result<u64> {
    let info = rustix::fs::statvfs(root.join("var/lib/sy-spark"))?;
    Ok(info.f_bavail.saturating_mul(info.f_frsize))
}

fn inventory_digest(connection: &rusqlite::Connection) -> Result<String> {
    let mut inventory = Vec::new();
    for query in [
        "SELECT id,repository,commit_sha,metadata_json FROM models ORDER BY id",
        "SELECT id,name,model_id,CAST(generation AS TEXT),desired_state,observed_state,metadata_json FROM instances ORDER BY id",
        "SELECT name,model_id FROM aliases ORDER BY name",
    ] {
        let mut statement = connection.prepare(query)?;
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|index| row.get::<_, String>(index))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        inventory.push(rows);
    }
    Ok(digest(&serde_json::to_vec(&inventory)?))
}

/// Recheck after draining, then again after stopping both state writers.
pub fn verify_sources(root: &Path, plan: &Plan) -> Result<()> {
    ensure!(
        digest(&super::release::read(
            &root.join("etc/sy/spark/models.toml"),
            1024 * 1024
        )?) == plan.model_catalog_sha256,
        "source model catalog changed after preflight"
    );
    for (relative, expected) in &plan.source_engines {
        ensure!(
            digest(&super::release::read(
                &root.join("etc/sy/spark/engines").join(
                    Path::new(relative)
                        .file_name()
                        .context("engine name missing")?
                ),
                1024 * 1024
            )?) == *expected,
            "source engine changed after preflight"
        );
    }
    let connection = rusqlite::Connection::open_with_flags(
        root.join("var/lib/sy-spark/state.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    connection.execute_batch("BEGIN")?;
    super::verify_database(&connection)?;
    super::emergency::validate(&root.join("var/lib/sy-spark"))?;
    ensure!(
        inventory_digest(&connection)? == plan.inventory_sha256,
        "source inventory changed after preflight"
    );
    Ok(())
}

/// `available` comes from the local kernel, never from an uploaded plan.
pub fn preflight(
    root: &Path,
    release: &Release,
    containers: &[serde_json::Value],
    available: u64,
) -> Result<Plan> {
    Publication::check_destinations(root)?;
    let data = root.join("var/lib/sy-spark");
    let metadata = fs::symlink_metadata(&data)?;
    ensure!(
        metadata.is_dir(),
        "legacy data root must be a real directory"
    );
    let uid = account(root, "sy-spark", false)?.context("legacy service user missing")?;
    let gid = account(root, "sy-spark", true)?.context("legacy service group missing")?;
    ensure!(
        uid != 0 && gid != 0 && metadata.uid() == uid && metadata.gid() == gid,
        "service numeric identity differs from data ownership"
    );
    ensure!(
        account(root, "sparkplane", false)?.is_none()
            && account(root, "sparkplane", true)?.is_none(),
        "destination service identity already exists"
    );
    for parent in [
        "var/lib",
        "opt",
        "etc",
        "etc/systemd/system",
        "etc/apparmor.d",
    ] {
        ensure!(
            fs::metadata(root.join(parent))?.dev() == metadata.dev(),
            "migration cannot relocate across filesystems"
        );
    }
    let agent = text(&root.join("etc/sy/spark-agent.toml"))?;
    let executor = text(&root.join("etc/sy/spark-executor.toml"))?;
    let converted = super::config::agent(&agent)?;
    let config: crate::spark::agent::AgentConfig = toml::from_str(&converted)?;
    let listen: SocketAddr = toml::from_str::<toml::Value>(&converted)?
        .get("listen")
        .and_then(toml::Value::as_str)
        .context("listen address missing")?
        .parse()?;
    ensure!(
        listen.port() == 9843 && !listen.ip().is_unspecified(),
        "migration requires the explicit appliance address on port 9843"
    );
    let converted_executor: crate::spark::executor::ExecutorConfig =
        toml::from_str(&super::config::executor(&executor)?)?;
    ensure!(
        converted_executor.agent_uid == uid,
        "executor peer UID differs from service UID"
    );
    ensure!(
        available
            >= config
                .resources
                .policy()
                .map_err(anyhow::Error::msg)?
                .disk_reserve_bytes
                .saturating_add(2 * 1024 * 1024 * 1024),
        "migration staging would cross disk reserve"
    );
    let mut engines = BTreeMap::new();
    let mut policies = Vec::new();
    let mut old_fingerprints = BTreeMap::new();
    let mut source_engines = BTreeMap::new();
    let model_catalog = text(&root.join("etc/sy/spark/models.toml"))?;
    release.verify_model_catalog(&model_catalog)?;
    let old_names = fs::read_dir(root.join("etc/sy/spark/engines"))?
        .map(|entry| entry.map(|e| e.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    ensure!(
        old_names
            .iter()
            .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "toml"))
            .count()
            == release.engines.len(),
        "source engine catalog contains policies absent from the signed release"
    );
    for (relative, bytes) in &release.engines {
        let name = Path::new(relative)
            .file_name()
            .context("engine name missing")?;
        let legacy = text(&root.join("etc/sy/spark/engines").join(name))?;
        let current = std::str::from_utf8(bytes)?;
        super::verify_engine_transition(&legacy, current)?;
        let policy = EnginePolicy::parse(current).map_err(anyhow::Error::msg)?;
        old_fingerprints.insert(
            policy.config().id.clone(),
            format!("sha256:{}", digest(legacy.as_bytes())),
        );
        source_engines.insert(relative.clone(), digest(legacy.as_bytes()));
        engines.insert(relative.clone(), current.into());
        policies.push(policy);
    }
    let connection = rusqlite::Connection::open_with_flags(
        data.join("state.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    connection.execute_batch("BEGIN")?;
    super::verify_database(&connection)?;
    super::emergency::validate(&data)?;
    let inventory_sha256 = inventory_digest(&connection)?;
    let mut statement =
        connection.prepare("SELECT id,repository,commit_sha,metadata_json FROM models")?;
    let models = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in models {
        let (id, repository, commit, json) = row?;
        let mut model: ModelDocument = serde_json::from_str(&json)?;
        ensure!(
            model.schema == "sy.spark.model/v1"
                && model.id == id
                && model.repository == repository
                && model.commit == commit,
            "legacy model primary fields and metadata disagree"
        );
        super::snapshot_path(&model.snapshot, &repository, &commit)?;
        if let Some(artifacts) = &mut model.artifacts {
            super::migrate_artifacts(artifacts)?;
        }
    }
    let mut active = Vec::new();
    let mut cache_keys = Vec::new();
    let mut statement = connection.prepare(
        "SELECT id,name,generation,desired_state,metadata_json FROM instances ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (id, name, generation, desired, json) = row?;
        let instance: InstanceDocument = serde_json::from_str(&json)?;
        ensure!(
            instance.schema == "sy.spark.instance/v2"
                && instance.id == id
                && instance.name == name
                && Some(instance.generation) == u64::try_from(generation).ok()
                && serde_json::to_value(instance.desired)? == desired
                && (instance.desired != InstanceDesiredState::Running
                    || (!instance.restart_suppressed && instance.quarantine.is_none())),
            "legacy instance is inconsistent or suppressed"
        );
        ensure!(
            crate::spark::wire::artifact_fingerprint(&instance.artifacts)
                .map_err(anyhow::Error::msg)?
                == instance.artifact_fingerprint,
            "legacy artifact fingerprint mismatch"
        );
        let mut artifacts = instance.artifacts.clone();
        super::migrate_artifacts(&mut artifacts)?;
        let fingerprint =
            crate::spark::wire::artifact_fingerprint(&artifacts).map_err(anyhow::Error::msg)?;
        let model_json: String = connection.query_row(
            "SELECT metadata_json FROM models WHERE id=?1",
            [&instance.model_id],
            |row| row.get(0),
        )?;
        let model: ModelDocument = serde_json::from_str(&model_json)?;
        ensure!(
            model.canonical == instance.model && model.commit == instance.model_commit,
            "instance model identity mismatch"
        );
        super::snapshot_path(&model.snapshot, &model.repository, &model.commit)?;
        if instance.desired != InstanceDesiredState::Running {
            continue;
        }
        let policy = policies
            .iter()
            .find(|p| p.config().id == instance.engine_id)
            .context("instance engine absent from release")?;
        ensure!(
            old_fingerprints.get(&instance.engine_id) == Some(&instance.engine_fingerprint),
            "legacy engine fingerprint differs from running instance"
        );
        let profile = policy
            .profile_for(None, &artifacts)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            profile.context_window == instance.context_window,
            "migration changes context window"
        );
        cache_keys.push(super::cache_keys(
            policy,
            &model.repository,
            &model.commit,
            &profile.id,
            &instance.artifact_fingerprint,
            &fingerprint,
        )?);
        let restart_operation: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE kind='instance.serve' AND target=?1 AND state='succeeded')",
            [&instance.name], |row| row.get(0))?;
        ensure!(
            restart_operation,
            "running instance has no durable successful serve operation"
        );
        ensure!(
            policy.config().family == "vllm"
                && profile.capabilities.iter().any(|c| c == "tool_calling"),
            "running engine is outside the supported migration qualification contract"
        );
        ensure!(
            !instance.name.is_empty()
                && instance.name.len() <= 96
                && instance
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
            "invalid instance route name"
        );
        ensure!(
            instance.healthy,
            "migration requires healthy running instances"
        );
        let matches: Vec<_> = containers
            .iter()
            .filter(|value| {
                value
                    .pointer("/Config/Labels/io.sy.spark.instance")
                    .and_then(serde_json::Value::as_str)
                    == Some(&instance.id)
            })
            .collect();
        ensure!(
            matches.len() == 1,
            "running instance has missing or ambiguous container identity"
        );
        let value = matches[0];
        let container = LegacyContainer {
            container_id: value
                .get("Id")
                .and_then(serde_json::Value::as_str)
                .context("Docker ID missing")?
                .into(),
            instance_id: instance.id.clone(),
            generation: instance.generation,
            image_digest: policy.config().image_digest.clone(),
            engine_id: instance.engine_id.clone(),
            engine_fingerprint: instance.engine_fingerprint.clone(),
            artifact_fingerprint: instance.artifact_fingerprint.clone(),
            model_repository: model.repository,
            model_commit: model.commit,
        };
        container.verify(value)?;
        super::container::verify_restart_policy(value)?;
        ensure!(
            value
                .pointer("/State/Running")
                .and_then(serde_json::Value::as_bool)
                == Some(true),
            "container must be running"
        );
        active.push(Active {
            before: instance,
            container,
            current_engine_fingerprint: policy.fingerprint().into(),
            current_artifact_fingerprint: fingerprint,
        });
    }
    ensure!(
        containers.len() == active.len(),
        "unaccounted managed container prevents migration"
    );
    Ok(Plan {
        schema: "sparkplane.appliance-migration/v1".into(),
        host: host_identity(root)?,
        release: digest(&release.manifest),
        executable: release.executable_sha256.clone(),
        uid,
        gid,
        listen,
        active,
        cache_keys,
        engines,
        agent_sha256: digest(agent.as_bytes()),
        executor_sha256: digest(executor.as_bytes()),
        inventory_sha256,
        source_engines,
        model_catalog_sha256: digest(model_catalog.as_bytes()),
        legacy_network: None,
    })
}

pub fn stage(root: &Path, work: &Path, plan: &Plan, release: &Release) -> Result<Publication> {
    let destination = work.join("stage");
    for relative in [
        "var/lib/sparkplane/ca",
        "var/lib/sparkplane/tls",
        "etc/sparkplane",
    ] {
        fs::create_dir_all(destination.join(relative))?;
    }
    for (from, to) in [
        (
            "var/lib/sy-spark/ca/ca-key.pem",
            "var/lib/sparkplane/ca/ca-key.pem",
        ),
        (
            "var/lib/sy-spark/ca/ca-cert.pem",
            "var/lib/sparkplane/ca/ca-cert.pem",
        ),
        (
            "var/lib/sy-spark/tls/server-key.pem",
            "var/lib/sparkplane/tls/server-key.pem",
        ),
        (
            "var/lib/sy-spark/tls/server-chain.pem",
            "var/lib/sparkplane/tls/server-chain.pem",
        ),
        (
            "etc/sy/spark-bootstrap-admin.credential",
            "etc/sparkplane/bootstrap-admin.credential",
        ),
        (
            "etc/sy/spark-hf-read.credential",
            "etc/sparkplane/hf-read.credential",
        ),
    ] {
        super::publication::write_private(
            &destination.join(to),
            &super::release::read(&root.join(from), 65536)?,
        )?;
    }
    release.stage(
        &destination,
        &plan.listen.ip().to_string(),
        text(&root.join("etc/hostname"))?.trim(),
    )?;
    let agent = super::config::agent(&text(&root.join("etc/sy/spark-agent.toml"))?)?;
    let executor = super::config::executor(&text(&root.join("etc/sy/spark-executor.toml"))?)?;
    for (name, contents) in [("agent.toml", agent), ("executor.toml", executor)] {
        let path = destination.join("etc/sparkplane").join(name);
        fs::write(&path, contents)?;
        fs::File::open(path)?.sync_all()?;
    }
    for entry in walkdir::WalkDir::new(destination.join("etc/sparkplane")).follow_links(false) {
        let entry = entry?;
        ensure!(
            !entry.file_type().is_symlink(),
            "staged configuration must not contain links"
        );
        rustix::fs::chown(
            entry.path(),
            Some(rustix::process::geteuid()),
            Some(rustix::fs::Gid::from_raw(plan.gid)),
        )?;
        if entry.file_type().is_dir() {
            fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o750))?;
        }
        fs::File::open(entry.path())?.sync_all()?;
    }
    Publication::capture(&destination)
}

impl Active {
    pub fn current(&self, inspected: &serde_json::Value) -> Result<LegacyContainer> {
        let mut identity = self.container.clone();
        identity.container_id = inspected
            .get("Id")
            .and_then(serde_json::Value::as_str)
            .context("current container ID missing")?
            .into();
        identity
            .engine_fingerprint
            .clone_from(&self.current_engine_fingerprint);
        identity
            .artifact_fingerprint
            .clone_from(&self.current_artifact_fingerprint);
        identity.verify_as(inspected, Namespace::Current)?;
        Ok(identity)
    }
}

pub fn active_directory() -> PathBuf {
    Path::new(BASE).join("active")
}
