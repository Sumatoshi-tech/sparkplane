//! Concrete journal actions. Only the platform boundary is substituted in tests.
use super::{
    appliance::{self, Plan},
    commands::Action,
    container::{LegacyContainer, Namespace, Network},
    docker::Containers,
    journal::Step,
    publication::{Publication, write_private},
    qualification::{Evidence, Gateway},
    runner::Actions,
    storage::Storage,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub trait Platform {
    fn action(&mut self, action: Action) -> Result<Vec<u8>>;
    fn inventory(&mut self, namespace: Namespace) -> Result<Vec<Value>>;
    fn stop(&mut self, container: &LegacyContainer) -> Result<()>;
    fn remove(&mut self, container: &LegacyContainer, namespace: Namespace) -> Result<()>;
    fn restore(&mut self, container: &LegacyContainer) -> Result<()>;
    fn network(&mut self, namespace: Namespace) -> Result<Option<Value>>;
    fn remove_network(&mut self, network: &Network, namespace: Namespace) -> Result<()>;
    fn healthy(&mut self, root: &Path, plan: &Plan, legacy: bool) -> Result<()>;
    fn qualify(&mut self, root: &Path, plan: &Plan, legacy: bool) -> Result<Evidence>;
    fn fallback(&mut self) -> Result<()>;
}

pub struct Linux {
    docker: Containers,
}
impl Linux {
    pub fn connect() -> Result<Self> {
        Ok(Self {
            docker: Containers::connect("/var/run/docker.sock")?,
        })
    }
}
impl Platform for Linux {
    fn action(&mut self, action: Action) -> Result<Vec<u8>> {
        action.run()
    }
    fn inventory(&mut self, namespace: Namespace) -> Result<Vec<Value>> {
        self.docker.inventory(namespace)
    }
    fn stop(&mut self, container: &LegacyContainer) -> Result<()> {
        self.docker.stop_exact(container, Namespace::Legacy)
    }
    fn remove(&mut self, container: &LegacyContainer, namespace: Namespace) -> Result<()> {
        self.docker.remove_exact(container, namespace)
    }
    fn restore(&mut self, container: &LegacyContainer) -> Result<()> {
        self.docker.restore_legacy(container)
    }
    fn network(&mut self, namespace: Namespace) -> Result<Option<Value>> {
        self.docker.network(namespace)
    }
    fn remove_network(&mut self, network: &Network, namespace: Namespace) -> Result<()> {
        self.docker.remove_network(network, namespace)
    }
    fn healthy(&mut self, root: &Path, plan: &Plan, legacy: bool) -> Result<()> {
        Gateway::load(root, plan, legacy)?.wait_healthy(plan, legacy)
    }
    fn qualify(&mut self, root: &Path, plan: &Plan, legacy: bool) -> Result<Evidence> {
        let namespace = if legacy {
            Namespace::Legacy
        } else {
            Namespace::Current
        };
        let inventory = self.inventory(namespace)?;
        ensure!(
            inventory.len() == plan.active.len(),
            "qualification container inventory changed"
        );
        Gateway::load(root, plan, legacy)?.qualify(plan, &inventory, namespace)
    }
    fn fallback(&mut self) -> Result<()> {
        Ok(crate::spark::install::ensure_http_fallback()?)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub plan: Plan,
    pub publication: Publication,
}

pub struct Host<P: Platform> {
    pub root: PathBuf,
    pub work: PathBuf,
    pub record: Record,
    pub platform: P,
}

impl<P: Platform> Host<P> {
    fn fence_present(&mut self) -> Result<bool> {
        let tables = self.platform.action(Action::ListTables)?;
        Ok(std::str::from_utf8(&tables)?
            .lines()
            .any(|line| line.trim() == "table inet sparkplane_migration"))
    }

    fn permit(&self) -> Result<()> {
        let path = self.root.join("run/sparkplane-migration-permit");
        if path.try_exists()? {
            ensure!(
                super::release::read(&path, 128)? == self.record.plan.release.as_bytes(),
                "migration permit identity changed"
            );
        } else {
            write_private(&path, self.record.plan.release.as_bytes())?;
        }
        Ok(())
    }

    pub fn restore_fence(&mut self) -> Result<()> {
        super::fence::install_guards(&self.root)?;
        self.platform.action(Action::ReloadUnits)?;
        if !self.fence_present()? {
            self.platform.action(Action::Fence)?;
        }
        self.platform.action(Action::SealTraffic)?;
        self.permit()
    }

    fn release_fence(&mut self) -> Result<()> {
        if self.fence_present()? {
            self.platform.action(Action::Unfence)?;
        }
        super::fence::remove_guards(&self.root)?;
        self.platform.action(Action::ReloadUnits)?;
        let permit = self.root.join("run/sparkplane-migration-permit");
        if permit.try_exists()? {
            fs::remove_file(permit)?;
            fs::File::open(self.root.join("run"))?.sync_all()?;
        }
        Ok(())
    }

    fn storage(&self) -> Result<Storage> {
        Ok(serde_json::from_slice(&super::release::read(
            &self.work.join("snapshot/storage.json"),
            1024 * 1024,
        )?)?)
    }

    fn rename_identity(&mut self, restore: bool) -> Result<()> {
        let (from, to) = if restore {
            ("sparkplane", "sy-spark")
        } else {
            ("sy-spark", "sparkplane")
        };
        for (group, numeric) in [(false, self.record.plan.uid), (true, self.record.plan.gid)] {
            match (
                appliance::account(&self.root, from, group)?,
                appliance::account(&self.root, to, group)?,
            ) {
                (Some(id), None) if id == numeric => {
                    self.platform.action(match (restore, group) {
                        (false, false) => Action::RenameUser,
                        (false, true) => Action::RenameGroup,
                        (true, false) => Action::RestoreUser,
                        (true, true) => Action::RestoreGroup,
                    })?;
                    ensure!(
                        appliance::account(&self.root, to, group)? == Some(numeric)
                            && appliance::account(&self.root, from, group)?.is_none(),
                        "service rename did not preserve numeric identity"
                    );
                }
                (None, Some(id)) if id == numeric => {}
                _ => anyhow::bail!("service identity conflict during migration"),
            }
        }
        Ok(())
    }

    fn remove_current(&mut self) -> Result<()> {
        let mut exact = Vec::new();
        for value in self.platform.inventory(Namespace::Current)? {
            let active = self
                .record
                .plan
                .active
                .iter()
                .find(|active| {
                    value
                        .pointer("/Config/Labels/io.sparkplane.instance")
                        .and_then(Value::as_str)
                        == Some(&active.before.id)
                })
                .context("unaccounted current container prevents recovery")?;
            exact.push(active.current(&value)?);
        }
        // Validate every identity before stopping even the first engine.
        for identity in exact {
            self.platform.remove(&identity, Namespace::Current)?;
        }
        ensure!(
            self.platform.inventory(Namespace::Current)?.is_empty(),
            "current container cleanup incomplete"
        );
        if let Some(value) = self.platform.network(Namespace::Current)? {
            let network = Network::capture(&value, Namespace::Current)?;
            self.platform.remove_network(&network, Namespace::Current)?;
        }
        Ok(())
    }
}

impl<P: Platform> Actions for Host<P> {
    fn apply(&mut self, step: Step) -> Result<()> {
        match step {
            Step::FenceTraffic => {
                ensure!(!self.fence_present()?, "traffic fence already exists");
                super::fence::install_guards(&self.root)?;
                self.platform.action(Action::ReloadUnits)?;
                self.platform.action(Action::Fence)?;
                self.permit()?;
            }
            Step::Drain => {
                let deadline = Instant::now() + Duration::from_secs(300);
                while !self.platform.action(Action::Connections)?.is_empty() {
                    ensure!(
                        Instant::now() < deadline,
                        "active client connections did not drain"
                    );
                    std::thread::sleep(Duration::from_millis(500));
                }
                self.platform.action(Action::SealTraffic)?;
                self.platform.healthy(&self.root, &self.record.plan, true)?;
                let baseline = self.platform.qualify(&self.root, &self.record.plan, true)?;
                write_private(
                    &self.work.join("before.json"),
                    &serde_json::to_vec(&baseline)?,
                )?;
            }
            Step::StopLegacyServices => {
                appliance::verify_sources(&self.root, &self.record.plan)?;
                for (name, expected) in [
                    ("spark-agent.toml", &self.record.plan.agent_sha256),
                    ("spark-executor.toml", &self.record.plan.executor_sha256),
                ] {
                    ensure!(
                        appliance::digest(&super::release::read(
                            &self.root.join("etc/sy").join(name),
                            1024 * 1024
                        )?) == *expected,
                        "source configuration changed after preflight"
                    );
                }
                self.platform.action(Action::StopOld)?;
            }
            Step::Snapshot => {
                appliance::verify_sources(&self.root, &self.record.plan)?;
                let snapshot = self.work.join("snapshot");
                fs::create_dir(&snapshot)?;
                let engines = self
                    .record
                    .plan
                    .engines
                    .values()
                    .map(|text| {
                        crate::spark::engine::EnginePolicy::parse(text).map_err(anyhow::Error::msg)
                    })
                    .collect::<Result<Vec<_>>>()?;
                let storage = Storage::prepare(
                    &self.root,
                    &snapshot,
                    &engines,
                    &self.record.plan.cache_keys,
                )?;
                write_private(
                    &snapshot.join("storage.json"),
                    &serde_json::to_vec(&storage)?,
                )?;
                fs::File::open(&self.work)?.sync_all()?;
            }
            Step::StopLegacyEngine => {
                for active in &self.record.plan.active {
                    self.platform.stop(&active.container)?;
                }
            }
            Step::Relocate => {
                self.storage()?
                    .activate(&self.root, &self.work.join("snapshot"))?;
                self.rename_identity(false)?;
                self.record
                    .publication
                    .activate(&self.root, &self.work.join("stage"))?;
            }
            Step::Activate => {
                self.platform.fallback()?;
                self.platform.action(Action::ConfineAgent)?;
                self.platform.action(Action::ConfineExecutor)?;
                self.platform.action(Action::ReloadUnits)?;
                self.platform.action(Action::StartNew)?;
            }
            Step::Qualify => {
                self.platform
                    .healthy(&self.root, &self.record.plan, false)?;
                let after = self
                    .platform
                    .qualify(&self.root, &self.record.plan, false)?;
                write_private(&self.work.join("after.json"), &serde_json::to_vec(&after)?)?;
                let before = serde_json::from_slice(&super::release::read(
                    &self.work.join("before.json"),
                    1024 * 1024,
                )?)?;
                super::qualification::compare(&before, &after)?;
                for active in &self.record.plan.active {
                    self.platform.remove(&active.container, Namespace::Legacy)?;
                }
                ensure!(
                    self.platform.inventory(Namespace::Legacy)?.is_empty(),
                    "legacy container cleanup incomplete"
                );
                if let Some(network) = &self.record.plan.legacy_network {
                    self.platform.remove_network(network, Namespace::Legacy)?;
                }
                self.platform
                    .healthy(&self.root, &self.record.plan, false)?;
            }
        }
        Ok(())
    }

    fn undo(&mut self, step: Step) -> Result<()> {
        match step {
            Step::Qualify | Step::Snapshot | Step::Drain => {}
            Step::Activate => {
                self.platform.action(Action::StopNew)?;
                self.remove_current()?;
            }
            Step::Relocate => {
                self.record
                    .publication
                    .restore(&self.root, &self.work.join("stage"))?;
                self.rename_identity(true)?;
                self.storage()?
                    .restore(&self.root, &self.work.join("snapshot"))?;
                self.platform.action(Action::ReloadUnits)?;
            }
            Step::StopLegacyEngine => {
                for active in &self.record.plan.active {
                    self.platform.restore(&active.container)?;
                }
            }
            Step::StopLegacyServices => {
                self.platform.action(Action::StartOld)?;
                self.platform.healthy(&self.root, &self.record.plan, true)?;
            }
            Step::FenceTraffic => {
                self.platform.healthy(&self.root, &self.record.plan, true)?;
                self.release_fence()?;
            }
        }
        Ok(())
    }

    fn open_traffic(&mut self) -> Result<()> {
        self.platform.action(Action::DisableOld)?;
        self.platform.action(Action::EnableNew)?;
        self.platform
            .healthy(&self.root, &self.record.plan, false)?;
        self.release_fence()
    }
}
