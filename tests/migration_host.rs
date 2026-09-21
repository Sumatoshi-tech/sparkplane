#![cfg(feature = "appliance")]
use sparkplane::migration::{
    appliance,
    commands::Action,
    container::{LegacyContainer, Namespace, Network},
    host::{Host, Platform, Record},
    journal::Journal,
    qualification::Evidence,
    release::Release,
    runner,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

struct FakePlatform {
    root: PathBuf,
    fence: bool,
    fail: Option<Action>,
    actions: Vec<Action>,
    old: Vec<serde_json::Value>,
    current: Vec<serde_json::Value>,
    replacement: Vec<serde_json::Value>,
}
impl Platform for FakePlatform {
    fn action(&mut self, action: Action) -> anyhow::Result<Vec<u8>> {
        self.actions.push(action);
        if self.fail == Some(action) {
            self.fail = None;
            anyhow::bail!("injected host failure");
        }
        match action {
            Action::Fence => self.fence = true,
            Action::Unfence => self.fence = false,
            Action::StartNew => self.current = self.replacement.clone(),
            Action::ListTables if self.fence => {
                return Ok(b"table inet sparkplane_migration\n".to_vec());
            }
            Action::RenameUser
            | Action::RenameGroup
            | Action::RestoreUser
            | Action::RestoreGroup => {
                let group = matches!(action, Action::RenameGroup | Action::RestoreGroup);
                let restore = matches!(action, Action::RestoreUser | Action::RestoreGroup);
                let path = self
                    .root
                    .join(if group { "etc/group" } else { "etc/passwd" });
                let (from, to) = if restore {
                    ("sparkplane:", "sy-spark:")
                } else {
                    ("sy-spark:", "sparkplane:")
                };
                fs::write(&path, fs::read_to_string(&path)?.replace(from, to))?;
            }
            _ => {}
        }
        Ok(Vec::new())
    }
    fn inventory(&mut self, namespace: Namespace) -> anyhow::Result<Vec<serde_json::Value>> {
        Ok(if namespace == Namespace::Legacy {
            &self.old
        } else {
            &self.current
        }
        .clone())
    }
    fn stop(&mut self, expected: &LegacyContainer) -> anyhow::Result<()> {
        for value in &mut self.old {
            if value["Id"] == expected.container_id {
                expected.verify(value)?;
                value["State"]["Running"] = false.into();
            }
        }
        Ok(())
    }
    fn remove(&mut self, expected: &LegacyContainer, namespace: Namespace) -> anyhow::Result<()> {
        let values = if namespace == Namespace::Legacy {
            &mut self.old
        } else {
            &mut self.current
        };
        for value in values.iter().filter(|v| v["Id"] == expected.container_id) {
            expected.verify_as(value, namespace)?;
        }
        values.retain(|v| v["Id"] != expected.container_id);
        Ok(())
    }
    fn restore(&mut self, expected: &LegacyContainer) -> anyhow::Result<()> {
        self.old = vec![inspect(expected, Namespace::Legacy)];
        Ok(())
    }
    fn network(&mut self, _: Namespace) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(None)
    }
    fn remove_network(&mut self, _: &Network, _: Namespace) -> anyhow::Result<()> {
        Ok(())
    }
    fn healthy(&mut self, _: &Path, _: &appliance::Plan, _: bool) -> anyhow::Result<()> {
        Ok(())
    }
    fn qualify(&mut self, _: &Path, plan: &appliance::Plan, _: bool) -> anyhow::Result<Evidence> {
        Ok(Evidence {
            decode_tokens_per_second: plan
                .active
                .iter()
                .map(|a| (a.before.id.clone(), vec![40.0, 41.0, 42.0]))
                .collect(),
        })
    }
    fn fallback(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

struct Fixture {
    root: tempfile::TempDir,
    host: Host<FakePlatform>,
    _db: sparkplane::spark::state::DbActor,
    release: Release,
}

fn legacy(text: &str) -> String {
    text.replace("sparkplane.engine/", "sy.spark.engine/")
        .replace("sparkplane.models/", "sy.spark.models/")
        .replace("sparkplane.agent/", "sy.spark.agent/")
        .replace("sparkplane.executor/", "sy.spark.executor/")
        .replace("/etc/sparkplane/agent.toml", "/etc/sy/spark-agent.toml")
        .replace("/etc/sparkplane/", "/etc/sy/spark/")
        .replace("/var/lib/sparkplane/", "/var/lib/sy-spark/")
        .replace("/run/sparkplane/", "/run/sy-spark/")
        .replace("/opt/sparkplane/", "/opt/sy-spark/")
        .replace("sparkplane-internal", "sy-spark-internal")
        .replace("sparkplane/", "sy-spark/")
}

fn fixture() -> Fixture {
    fixture_with_instance(false)
}

fn fixture_with_instance(populated: bool) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    for name in [
        "etc/sy/spark/engines",
        "etc/systemd/system",
        "etc/apparmor.d",
        "opt",
        "run",
        "var/lib/sy-spark/huggingface",
        "var/lib/sy-spark/ca",
        "var/lib/sy-spark/tls",
        "work",
    ] {
        fs::create_dir_all(root.path().join(name)).unwrap();
    }
    let privileged = rustix::process::geteuid().is_root();
    let uid = if privileged {
        996
    } else {
        rustix::process::geteuid().as_raw()
    };
    let gid = if privileged {
        996
    } else {
        rustix::process::getegid().as_raw()
    };
    if privileged {
        rustix::fs::chown(
            root.path().join("var/lib/sy-spark"),
            Some(rustix::fs::Uid::from_raw(uid)),
            Some(rustix::fs::Gid::from_raw(gid)),
        )
        .unwrap();
    }
    fs::write(
        root.path().join("etc/passwd"),
        format!("sy-spark:x:{uid}:{gid}::/nonexistent:/usr/sbin/nologin\n"),
    )
    .unwrap();
    fs::write(
        root.path().join("etc/group"),
        format!("sy-spark:x:{gid}:\n"),
    )
    .unwrap();
    fs::write(root.path().join("etc/machine-id"), "a".repeat(32)).unwrap();
    fs::write(root.path().join("etc/hostname"), "fixture").unwrap();
    for relative in [
        "ca/ca-key.pem",
        "ca/ca-cert.pem",
        "tls/server-key.pem",
        "tls/server-chain.pem",
    ] {
        fs::write(
            root.path().join("var/lib/sy-spark").join(relative),
            "private fixture",
        )
        .unwrap();
    }
    for name in [
        "spark-bootstrap-admin.credential",
        "spark-hf-read.credential",
    ] {
        fs::write(root.path().join("etc/sy").join(name), "fixture credential").unwrap();
    }
    fs::write(
        root.path().join("etc/sy/spark-agent.toml"),
        legacy(include_str!("../configs/sparkplane/agent.toml")),
    )
    .unwrap();
    fs::write(
        root.path().join("etc/sy/spark/models.toml"),
        legacy(include_str!("../configs/sparkplane/models.toml")),
    )
    .unwrap();
    fs::write(
        root.path().join("etc/sy/spark-executor.toml"),
        legacy(
            &include_str!("../configs/sparkplane/executor.toml").replace("996", &uid.to_string()),
        ),
    )
    .unwrap();
    fs::write(
        root.path().join("var/lib/sy-spark/huggingface/weights"),
        b"unaltered",
    )
    .unwrap();
    let db = sparkplane::spark::state::DbActor::open(
        root.path().join("var/lib/sy-spark/state.sqlite3"),
        root.path().join("var/lib/sy-spark/backups"),
        8,
        2,
        secrecy::SecretString::from("fixture"),
    )
    .unwrap();
    let bundle = root.path().join("bundle");
    fs::create_dir_all(bundle.join("configs/sparkplane/engines")).unwrap();
    let mut files = BTreeMap::from([
        (
            "sparkplane-aarch64".to_owned(),
            b"signed fixture executable".to_vec(),
        ),
        (
            "configs/sparkplane/models.toml".into(),
            include_bytes!("../configs/sparkplane/models.toml").to_vec(),
        ),
    ]);
    for entry in
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("configs/sparkplane/engines"))
            .unwrap()
    {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|s| s == "toml") {
            let bytes = fs::read(entry.path()).unwrap();
            fs::write(
                root.path()
                    .join("etc/sy/spark/engines")
                    .join(entry.file_name()),
                legacy(std::str::from_utf8(&bytes).unwrap()),
            )
            .unwrap();
            files.insert(
                format!(
                    "configs/sparkplane/engines/{}",
                    entry.file_name().to_str().unwrap()
                ),
                bytes,
            );
        }
    }
    let manifest = files
        .iter()
        .map(|(name, bytes)| format!("{}  {name}\n", appliance::digest(bytes)))
        .collect::<String>();
    for (name, bytes) in files {
        fs::write(bundle.join(name), bytes).unwrap();
    }
    let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let signature = minisign::sign(
        None,
        &keys.sk,
        std::io::Cursor::new(manifest.as_bytes()),
        None,
        None,
    )
    .unwrap()
    .to_string();
    fs::write(bundle.join("SHA256SUMS"), manifest).unwrap();
    fs::write(bundle.join("SHA256SUMS.minisig"), signature).unwrap();
    let release = Release::load(&bundle, &keys.pk.to_base64()).unwrap();
    assert!(appliance::preflight(root.path(), &release, &[], 1).is_err());
    let old = if populated {
        populate(root.path())
    } else {
        vec![]
    };
    let plan = appliance::preflight(root.path(), &release, &old, u64::MAX).unwrap();
    for (old, _) in &plan.cache_keys {
        let cache = root.path().join("var/lib/sy-spark/compile-cache").join(old);
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("compiled"), b"qualified warm cache").unwrap();
    }
    let replacement = plan
        .active
        .iter()
        .map(|active| {
            let mut container = active.container.clone();
            container.container_id = "d".repeat(64);
            container
                .engine_fingerprint
                .clone_from(&active.current_engine_fingerprint);
            container
                .artifact_fingerprint
                .clone_from(&active.current_artifact_fingerprint);
            inspect(&container, Namespace::Current)
        })
        .collect();
    let work = root.path().join("work");
    let publication = appliance::stage(root.path(), &work, &plan, &release).unwrap();
    let host = Host {
        root: root.path().into(),
        work,
        record: Record { plan, publication },
        platform: FakePlatform {
            root: root.path().into(),
            fence: false,
            fail: None,
            actions: vec![],
            old,
            current: vec![],
            replacement,
        },
    };
    Fixture {
        root,
        host,
        _db: db,
        release,
    }
}

fn inspect(identity: &LegacyContainer, namespace: Namespace) -> serde_json::Value {
    let labels: BTreeMap<_, _> = identity
        .labels()
        .into_iter()
        .map(|(key, value)| (key.replace("io.sy.spark", namespace.prefix()), value))
        .collect();
    serde_json::json!({"Id":identity.container_id,"Image":identity.image_digest,
        "Name":format!("/{}-{}-g{}",namespace.name(),identity.instance_id,identity.generation),
        "Config":{"Labels":labels},"State":{"Running":true},"HostConfig":{"RestartPolicy":{"Name":"no"}}})
}

fn populate(root: &Path) -> Vec<serde_json::Value> {
    use sparkplane::spark::{engine::EnginePolicy, wire};
    let policy = EnginePolicy::parse(include_str!(
        "../configs/sparkplane/engines/vllm-qwen38-mmap.toml"
    ))
    .unwrap();
    let engine = fs::read(root.join("etc/sy/spark/engines/vllm-qwen38-mmap.toml")).unwrap();
    let artifacts: wire::ModelArtifactsDocument = serde_json::from_value(serde_json::json!({
        "schema":"sy.spark.model-artifacts/v2","format":"safetensors",
        "primary":{"path":"model.safetensors","bytes":8,"sha256":null},"auxiliary":[],
        "quantization":"NVFP4","capabilities":["text_generation","tool_calling"],
        "configured_alias":null,"engine_profile":"qwen3.8-flash-next-nvfp4"
    }))
    .unwrap();
    let model = serde_json::json!({"schema":"sy.spark.model/v1","id":"model-id",
        "canonical":format!("huggingface:owner/model@{}#sha256:{}", "a".repeat(40), "b".repeat(64)),
        "repository":"owner/model","commit":"a".repeat(40),
        "snapshot":format!("/var/lib/sy-spark/huggingface/models--owner--model/snapshots/{}", "a".repeat(40)),
        "artifacts":artifacts,"logical_bytes":8,"unique_bytes":8,"aliases":["my-model"],
        "active_instances":[],"transport":"fixture","verified_at":"2026-09-21T00:00:00Z","gated":false,"license":"MIT"});
    let instance: wire::InstanceDocument = serde_json::from_value(serde_json::json!({
        "schema":"sy.spark.instance/v2","id":format!("i_{}", "c".repeat(32)),"name":"qwen38-vllm",
        "model_id":"model-id","model":model["canonical"],"model_commit":model["commit"],
        "engine_id":policy.config().id,"engine_fingerprint":format!("sha256:{}", appliance::digest(&engine)),
        "artifact_fingerprint":wire::artifact_fingerprint(&artifacts).unwrap(),"artifacts":artifacts,
        "objective":"agent","resources":{"image_bytes":1,"startup_peak_bytes":2,"steady_peak_bytes":1,"compile_cache_bytes":1},
        "context_window":262144,"generation":8,"desired":"running","observed":"healthy",
        "endpoint":null,"healthy":true,"started_at":"now","last_failure":null
    })).unwrap();
    let db = rusqlite::Connection::open(root.join("var/lib/sy-spark/state.sqlite3")).unwrap();
    db.execute(
        "INSERT INTO models VALUES('model-id','owner/model',?1,?2)",
        [&"a".repeat(40), &model.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO instances VALUES(?1,'qwen38-vllm','model-id',8,'running','healthy',?2)",
        [&instance.id, &serde_json::to_string(&instance).unwrap()],
    )
    .unwrap();
    db.execute("INSERT INTO operations VALUES('01M303PWWXR02FK2H7XN15NV7B','instance.serve','admin','qwen38-vllm','succeeded','{}','now','now',NULL,NULL)", []).unwrap();
    vec![inspect(
        &LegacyContainer {
            container_id: "a".repeat(64),
            instance_id: instance.id,
            generation: 8,
            image_digest: policy.config().image_digest.clone(),
            engine_id: instance.engine_id,
            engine_fingerprint: instance.engine_fingerprint,
            artifact_fingerprint: instance.artifact_fingerprint,
            model_repository: "owner/model".into(),
            model_commit: "a".repeat(40),
        },
        Namespace::Legacy,
    )]
}

#[test]
fn preflight_preserves_stopped_historical_engine_settings() {
    let fixture = fixture_with_instance(true);
    let db = rusqlite::Connection::open(fixture.root.path().join("var/lib/sy-spark/state.sqlite3"))
        .unwrap();
    db.execute("UPDATE instances SET desired_state='stopped', observed_state='absent', metadata_json=json_set(metadata_json,'$.desired','stopped','$.observed','absent','$.healthy',json('false'),'$.engine_fingerprint',?1,'$.context_window',8192)", [format!("sha256:{}", "e".repeat(64))]).unwrap();
    let plan = appliance::preflight(fixture.root.path(), &fixture.release, &[], u64::MAX).unwrap();
    assert!(plan.active.is_empty() && plan.cache_keys.is_empty());
}

#[test]
fn preflight_accepts_retired_policies_only_for_stopped_history() {
    let fixture = fixture_with_instance(true);
    let db = rusqlite::Connection::open(fixture.root.path().join("var/lib/sy-spark/state.sqlite3"))
        .unwrap();
    db.execute(
        "UPDATE instances SET metadata_json=json_set(metadata_json,'$.engine_id','retired-policy')",
        [],
    )
    .unwrap();
    assert!(appliance::preflight(fixture.root.path(), &fixture.release, &[], u64::MAX).is_err());
    db.execute("UPDATE instances SET desired_state='stopped', observed_state='absent', metadata_json=json_set(metadata_json,'$.desired','stopped','$.observed','absent','$.healthy',json('false'))", []).unwrap();
    assert!(appliance::preflight(fixture.root.path(), &fixture.release, &[], u64::MAX).is_ok());
}

#[test]
fn preflight_accepts_the_managed_unless_stopped_policy_but_rejects_always() {
    let fixture = fixture_with_instance(true);
    let mut containers = fixture.host.platform.old.clone();
    containers[0]["HostConfig"]["RestartPolicy"]["Name"] = "unless-stopped".into();
    assert!(
        appliance::preflight(fixture.root.path(), &fixture.release, &containers, u64::MAX).is_ok()
    );
    containers[0]["HostConfig"]["RestartPolicy"]["Name"] = "always".into();
    assert!(
        appliance::preflight(fixture.root.path(), &fixture.release, &containers, u64::MAX).is_err()
    );
}

#[test]
fn running_instances_still_require_the_exact_engine_fingerprint() {
    let fixture = fixture_with_instance(true);
    let db = rusqlite::Connection::open(fixture.root.path().join("var/lib/sy-spark/state.sqlite3"))
        .unwrap();
    db.execute(
        "UPDATE instances SET metadata_json=json_set(metadata_json,'$.engine_fingerprint',?1)",
        [format!("sha256:{}", "e".repeat(64))],
    )
    .unwrap();
    let error = appliance::preflight(
        fixture.root.path(),
        &fixture.release,
        &fixture.host.platform.old,
        u64::MAX,
    )
    .err()
    .unwrap();
    assert!(
        error
            .to_string()
            .contains("legacy engine fingerprint differs")
    );
}

#[test]
fn stopped_suppression_is_preserved_without_reactivating_the_instance() {
    let fixture = fixture_with_instance(true);
    let source = fixture.root.path().join("var/lib/sy-spark/state.sqlite3");
    let db = rusqlite::Connection::open(&source).unwrap();
    db.execute("UPDATE instances SET desired_state='stopped', observed_state='absent', metadata_json=json_set(metadata_json,'$.desired','stopped','$.observed','absent','$.healthy',json('false'),'$.restart_suppressed',json('true'))", []).unwrap();
    appliance::preflight(fixture.root.path(), &fixture.release, &[], u64::MAX).unwrap();
    let destination = fixture.root.path().join("suppressed.sqlite3");
    sparkplane::migration::stage_database(&source, &destination, &[]).unwrap();
    let saved: (String, bool) = rusqlite::Connection::open(destination).unwrap().query_row("SELECT desired_state,json_extract(metadata_json,'$.restart_suppressed') FROM instances", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    assert_eq!(saved, ("stopped".into(), true));
}

#[test]
fn snapshot_retains_stopped_history_without_a_current_engine_policy() {
    let fixture = fixture_with_instance(true);
    let source = fixture.root.path().join("var/lib/sy-spark/state.sqlite3");
    let db = rusqlite::Connection::open(&source).unwrap();
    let historical = format!("sha256:{}", "e".repeat(64));
    db.execute("UPDATE instances SET desired_state='stopped', observed_state='absent', metadata_json=json_set(metadata_json,'$.desired','stopped','$.observed','absent','$.healthy',json('false'),'$.engine_fingerprint',?1,'$.context_window',8192)", [&historical]).unwrap();
    let destination = fixture.root.path().join("history.sqlite3");
    sparkplane::migration::stage_database(&source, &destination, &[]).unwrap();
    let snapshot = rusqlite::Connection::open(destination).unwrap();
    let preserved: (String, i64) = snapshot.query_row("SELECT json_extract(metadata_json,'$.engine_fingerprint'), json_extract(metadata_json,'$.context_window') FROM instances", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    assert_eq!(preserved, (historical, 8192));
}

#[test]
fn running_model_cutover_preserves_generation_context_image_and_warm_cache_inode() {
    use std::os::unix::fs::MetadataExt;
    let mut fixture = fixture_with_instance(true);
    let (old_key, new_key) = fixture.host.record.plan.cache_keys[0].clone();
    let old_cache = fixture
        .root
        .path()
        .join("var/lib/sy-spark/compile-cache")
        .join(old_key);
    let inode = fs::metadata(old_cache.join("compiled")).unwrap().ino();
    let mut journal = Journal::open(
        &fixture.host.work,
        &fixture.host.record.plan.host,
        &fixture.host.record.plan.release,
    )
    .unwrap();
    runner::run(&mut journal, &mut fixture.host).unwrap();
    assert!(fixture.host.platform.old.is_empty());
    assert_eq!(fixture.host.platform.current.len(), 1);
    let db =
        rusqlite::Connection::open(fixture.root.path().join("var/lib/sparkplane/state.sqlite3"))
            .unwrap();
    let json: String = db
        .query_row("SELECT metadata_json FROM instances", [], |r| r.get(0))
        .unwrap();
    let instance: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(instance["generation"], 8);
    assert_eq!(instance["context_window"], 262144);
    assert_eq!(
        instance["engine_fingerprint"],
        fixture.host.record.plan.active[0].current_engine_fingerprint
    );
    let cache = fixture
        .root
        .path()
        .join("var/lib/sparkplane/compile-cache")
        .join(new_key)
        .join("compiled");
    assert_eq!(fs::metadata(&cache).unwrap().ino(), inode);
    assert_eq!(fs::read(cache).unwrap(), b"qualified warm cache");
}

#[test]
fn actual_host_transaction_commits_without_docker_or_model_data_replacement() {
    let mut fixture = fixture();
    let mut journal = Journal::open(
        &fixture.host.work,
        &fixture.host.record.plan.host,
        &fixture.host.record.plan.release,
    )
    .unwrap();
    runner::run(&mut journal, &mut fixture.host).unwrap();
    assert!(journal.committed().unwrap());
    assert!(!fixture.host.platform.fence);
    assert_eq!(
        fs::read(
            fixture
                .root
                .path()
                .join("var/lib/sparkplane/huggingface/weights")
        )
        .unwrap(),
        b"unaltered"
    );
    assert!(
        fixture
            .root
            .path()
            .join("opt/sparkplane/current/sparkplane")
            .is_file()
    );
    assert!(
        fixture
            .root
            .path()
            .join("etc/sparkplane/agent.toml")
            .is_file()
    );
    assert!(runner::recover(&mut journal, &mut fixture.host).is_err());
}

#[test]
fn source_inventory_drift_is_rejected_before_services_stop() {
    let mut fixture = fixture();
    let connection =
        rusqlite::Connection::open(fixture.root.path().join("var/lib/sy-spark/state.sqlite3"))
            .unwrap();
    connection
        .execute(
            "INSERT INTO models VALUES('unexpected','owner/model',?1,'{}')",
            ["a".repeat(40)],
        )
        .unwrap();
    let mut journal = Journal::open(
        &fixture.host.work,
        &fixture.host.record.plan.host,
        &fixture.host.record.plan.release,
    )
    .unwrap();
    assert!(runner::run(&mut journal, &mut fixture.host).is_err());
    assert!(!fixture.host.platform.actions.contains(&Action::StopOld));
    runner::recover(&mut journal, &mut fixture.host).unwrap();
}

#[test]
fn preflight_rejects_catalog_changes_and_unreferenced_invalid_models() {
    let fixture = fixture();
    assert!(fixture.release.verify_model_catalog("").is_err());
    let model_catalog =
        fs::read_to_string(fixture.root.path().join("etc/sy/spark/models.toml")).unwrap();
    assert!(
        fixture
            .release
            .verify_model_catalog(&model_catalog.replace("ornith-ai/", "different/"))
            .is_err()
    );
    let connection =
        rusqlite::Connection::open(fixture.root.path().join("var/lib/sy-spark/state.sqlite3"))
            .unwrap();
    connection
        .execute(
            "INSERT INTO models VALUES('unreferenced','owner/model',?1,'{}')",
            ["a".repeat(40)],
        )
        .unwrap();
    assert!(appliance::preflight(fixture.root.path(), &fixture.release, &[], u64::MAX).is_err());
}

#[test]
fn staging_uses_verified_bytes_and_loading_rejects_tampering_links_and_wrong_signers() {
    let fixture = fixture();
    let bundle = fixture.root.path().join("bundle");
    let wrong = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    assert!(Release::load(&bundle, &wrong.pk.to_base64()).is_err());
    let public_key = fs::read_to_string(
        fixture
            .host
            .work
            .join("stage/opt/sparkplane/current/minisign.pub"),
    )
    .unwrap();
    let public_key = public_key
        .lines()
        .find(|line| !line.starts_with("untrusted comment:") && !line.is_empty())
        .unwrap();
    fs::write(bundle.join("sparkplane-aarch64"), b"tampered").unwrap();
    assert!(Release::load(&bundle, public_key).is_err());
    assert_eq!(
        fs::read(
            fixture
                .host
                .work
                .join("stage/opt/sparkplane/current/sparkplane")
        )
        .unwrap(),
        b"signed fixture executable"
    );
    fs::rename(bundle.join("sparkplane-aarch64"), bundle.join("replaced")).unwrap();
    std::os::unix::fs::symlink(bundle.join("replaced"), bundle.join("sparkplane-aarch64")).unwrap();
    assert!(Release::load(&bundle, public_key).is_err());
}

#[test]
fn engine_policy_drift_is_rejected_before_services_stop() {
    let mut fixture = fixture();
    let name = Path::new(fixture.host.record.plan.engines.keys().next().unwrap())
        .file_name()
        .unwrap();
    fs::write(
        fixture.root.path().join("etc/sy/spark/engines").join(name),
        "changed",
    )
    .unwrap();
    let mut journal = Journal::open(
        &fixture.host.work,
        &fixture.host.record.plan.host,
        &fixture.host.record.plan.release,
    )
    .unwrap();
    assert!(runner::run(&mut journal, &mut fixture.host).is_err());
    assert!(!fixture.host.platform.actions.contains(&Action::StopOld));
    runner::recover(&mut journal, &mut fixture.host).unwrap();
}

#[test]
fn partial_service_account_and_activation_failures_restore_the_original_layout() {
    for fail in [
        Action::StopOld,
        Action::RenameGroup,
        Action::ConfineAgent,
        Action::StartNew,
    ] {
        let mut fixture = fixture_with_instance(true);
        fixture.host.platform.fail = Some(fail);
        let original =
            fs::read(fixture.root.path().join("var/lib/sy-spark/state.sqlite3")).unwrap();
        let mut journal = Journal::open(
            &fixture.host.work,
            &fixture.host.record.plan.host,
            &fixture.host.record.plan.release,
        )
        .unwrap();
        assert!(runner::run(&mut journal, &mut fixture.host).is_err());
        fixture.host.restore_fence().unwrap();
        runner::recover(&mut journal, &mut fixture.host).unwrap();
        assert!(!fixture.host.platform.fence);
        assert!(
            fs::read(fixture.root.path().join("var/lib/sy-spark/state.sqlite3")).unwrap()
                == original
        );
        assert!(!fixture.root.path().join("var/lib/sparkplane").exists());
        assert!(!fixture.root.path().join("opt/sparkplane").exists());
        let (old_cache, _) = &fixture.host.record.plan.cache_keys[0];
        assert_eq!(
            fs::read(
                fixture
                    .root
                    .path()
                    .join("var/lib/sy-spark/compile-cache")
                    .join(old_cache)
                    .join("compiled")
            )
            .unwrap(),
            b"qualified warm cache"
        );
        assert!(fixture.host.platform.current.is_empty());
        assert_eq!(fixture.host.platform.old[0]["State"]["Running"], true);
        assert!(
            appliance::account(fixture.root.path(), "sy-spark", false)
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn failure_opening_traffic_after_commit_cannot_restore_old_data() {
    let mut fixture = fixture();
    fixture.host.platform.fail = Some(Action::Unfence);
    let mut journal = Journal::open(
        &fixture.host.work,
        &fixture.host.record.plan.host,
        &fixture.host.record.plan.release,
    )
    .unwrap();
    assert!(runner::run(&mut journal, &mut fixture.host).is_err());
    assert!(journal.committed().unwrap());
    assert!(runner::recover(&mut journal, &mut fixture.host).is_err());
    runner::run(&mut journal, &mut fixture.host).unwrap();
    assert!(!fixture.host.platform.fence);
}
