//! Durable migration intent. An unfinished action must be recovered, never skipped.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    FenceTraffic,
    Drain,
    StopLegacyServices,
    Snapshot,
    StopLegacyEngine,
    Relocate,
    Activate,
    Qualify,
}
pub const STEPS: [Step; 8] = [
    Step::FenceTraffic,
    Step::Drain,
    Step::StopLegacyServices,
    Step::Snapshot,
    Step::StopLegacyEngine,
    Step::Relocate,
    Step::Activate,
    Step::Qualify,
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema: String,
    host: String,
    release: String,
    completed: Vec<Step>,
    pending: Option<Step>,
    committed: bool,
    rolling_back: bool,
}

pub struct Journal {
    path: PathBuf,
    state: State,
    _lock: fs::File,
    poisoned: bool,
}

impl Journal {
    pub fn open(directory: &Path, host: &str, release: &str) -> Result<Self> {
        ensure!(
            [host, release]
                .iter()
                .all(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())),
            "invalid migration identity"
        );
        let metadata = fs::symlink_metadata(directory)?;
        ensure!(
            directory.is_absolute()
                && metadata.is_dir()
                && metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.mode() & 0o022 == 0,
            "migration journal directory must be owned and not shared-writable"
        );
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join("migration.lock"))?;
        lock.try_lock()?;
        let path = directory.join("migration.json");
        ensure!(
            !path.is_symlink(),
            "migration journal must not be a symlink"
        );
        let state = if path.try_exists()? {
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                metadata.is_file() && metadata.len() <= 4096,
                "invalid migration journal"
            );
            serde_json::from_slice(&fs::read(&path)?)?
        } else {
            State {
                schema: "sparkplane.migration-journal/v1".into(),
                host: host.into(),
                release: release.into(),
                completed: vec![],
                pending: None,
                committed: false,
                rolling_back: false,
            }
        };
        ensure!(
            state.schema == "sparkplane.migration-journal/v1"
                && state.host == host
                && state.release == release,
            "migration journal identity mismatch"
        );
        ensure!(
            STEPS.starts_with(&state.completed)
                && state
                    .pending
                    .is_none_or(|step| STEPS.get(state.completed.len()) == Some(&step))
                && (!state.committed
                    || (state.completed == STEPS
                        && state.pending.is_none()
                        && !state.rolling_back)),
            "migration journal contains an invalid action history"
        );
        let mut journal = Self {
            path,
            state,
            _lock: lock,
            poisoned: false,
        };
        journal.save()?;
        Ok(journal)
    }

    pub fn pending(&self) -> Option<Step> {
        self.state.pending
    }

    /// A signed replacement may only affect health checks after state restoration.
    pub fn restored_checkpoint(&self) -> bool {
        !self.poisoned
            && !self.state.committed
            && self.state.rolling_back
            && self.state.pending.is_none()
            && self.state.completed.len() <= 3
    }

    pub fn committed(&self) -> Result<bool> {
        self.require_durable()?;
        Ok(self.state.committed)
    }

    pub fn next(&self) -> Result<Option<Step>> {
        self.require_durable()?;
        ensure!(
            self.state.pending.is_none() && !self.state.rolling_back,
            "recover interrupted migration before continuing"
        );
        Ok(STEPS.get(self.state.completed.len()).copied())
    }

    /// Persist the no-rollback boundary BEFORE permitting external mutations.
    pub fn commit(&mut self) -> Result<()> {
        self.require_durable()?;
        ensure!(
            self.state.completed == STEPS
                && self.state.pending.is_none()
                && !self.state.rolling_back,
            "migration must pass every acceptance step before commit"
        );
        self.state.committed = true;
        self.save()
    }

    /// `undo` must tolerate an action already reversed before an interrupted fsync.
    pub fn recover(&mut self, mut undo: impl FnMut(Step) -> Result<()>) -> Result<()> {
        self.require_durable()?;
        ensure!(
            !self.state.committed,
            "committed migrations cannot restore stale state"
        );
        self.state.rolling_back = true;
        self.save()?;
        if let Some(step) = self.state.pending {
            undo(step)?;
            self.state.pending = None;
            self.save()?;
        }
        while let Some(step) = self.state.completed.last().copied() {
            undo(step)?;
            self.state.completed.pop();
            self.save()?;
        }
        self.state.rolling_back = false;
        self.save()
    }

    pub fn finish(&mut self, step: Step) -> Result<()> {
        self.require_durable()?;
        ensure!(
            !self.state.rolling_back && self.state.pending == Some(step),
            "cannot finish an action without its durable intent"
        );
        self.state.completed.push(step);
        self.state.pending = None;
        self.save()
    }

    pub fn begin(&mut self, step: Step) -> Result<()> {
        self.require_durable()?;
        ensure!(
            !self.state.committed && !self.state.rolling_back && self.state.pending.is_none(),
            "recover pending migration before continuing"
        );
        ensure!(
            STEPS.get(self.state.completed.len()) == Some(&step),
            "migration actions must follow the fixed sequence"
        );
        self.state.pending = Some(step);
        self.save()
    }

    fn require_durable(&self) -> Result<()> {
        ensure!(
            !self.poisoned,
            "journal persistence failed; reopen the durable journal before recovery"
        );
        Ok(())
    }

    fn save(&mut self) -> Result<()> {
        self.poisoned = true;
        let parent = self.path.parent().expect("journal has a parent");
        let stage_path = parent.join(format!(".migration-{}", uuid::Uuid::new_v4()));
        let mut stage = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&stage_path)?;
        serde_json::to_writer(&mut stage, &self.state)?;
        stage.flush()?;
        stage.sync_all()?;
        fs::rename(stage_path, &self.path)?;
        fs::File::open(parent)?.sync_all()?;
        self.poisoned = false;
        Ok(())
    }
}
