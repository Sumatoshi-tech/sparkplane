//! Publish only staged, verified files; preserve originals on rollback.
use super::relocation::{FileIdentity, Identity, move_file, relocate};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

const TREES: [&str; 2] = ["opt/sparkplane", "etc/sparkplane"];
const ASSETS: [&str; 5] = [
    "etc/systemd/system/sparkplane-agent.service",
    "etc/systemd/system/sparkplane-executor.service",
    "etc/systemd/system/sparkplane.target",
    "etc/apparmor.d/sparkplane-agent",
    "etc/apparmor.d/sparkplane-executor",
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publication {
    trees: Vec<Identity>,
    files: Vec<FileIdentity>,
}

impl Publication {
    pub fn check_destinations(root: &Path) -> Result<()> {
        for relative in TREES.into_iter().chain(ASSETS) {
            let path = root.join(relative);
            ensure!(
                !path.try_exists()? && !path.is_symlink(),
                "migration destination conflict: {relative}"
            );
            let parent = path.parent().expect("fixed destination parent");
            ensure!(
                !parent.is_symlink() && parent.is_dir(),
                "migration destination parent is not a real directory"
            );
        }
        Ok(())
    }

    pub fn capture(stage: &Path) -> Result<Self> {
        Ok(Self {
            trees: TREES
                .iter()
                .map(|path| Identity::read(&stage.join(path)))
                .collect::<Result<_>>()?,
            files: ASSETS
                .iter()
                .map(|path| FileIdentity::read(&stage.join(path)))
                .collect::<Result<_>>()?,
        })
    }

    pub fn activate(&self, root: &Path, stage: &Path) -> Result<()> {
        ensure!(
            self.trees.len() == TREES.len() && self.files.len() == ASSETS.len(),
            "invalid staged publication inventory"
        );
        for (path, id) in TREES.iter().zip(&self.trees) {
            relocate(&stage.join(path), &root.join(path), id)?;
        }
        for (path, id) in ASSETS.iter().zip(&self.files) {
            move_file(&stage.join(path), &root.join(path), id)?;
        }
        Ok(())
    }

    pub fn restore(&self, root: &Path, stage: &Path) -> Result<()> {
        ensure!(
            self.trees.len() == TREES.len() && self.files.len() == ASSETS.len(),
            "invalid recovery publication inventory"
        );
        for (path, id) in ASSETS.iter().zip(&self.files).rev() {
            if root.join(path).try_exists()? {
                move_file(&root.join(path), &stage.join(path), id)?;
            } else {
                ensure!(
                    FileIdentity::read(&stage.join(path))? == *id,
                    "staged asset identity changed"
                );
            }
        }
        for (path, id) in TREES.iter().zip(&self.trees).rev() {
            if root.join(path).try_exists()? {
                relocate(&root.join(path), &stage.join(path), id)?;
            } else {
                ensure!(
                    Identity::read(&stage.join(path))? == *id,
                    "staged tree identity changed"
                );
            }
        }
        Ok(())
    }
}

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let parent = path.parent().expect("private file has parent");
    let stage = parent.join(format!(".stage-{}", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stage)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        &stage,
        rustix::fs::CWD,
        path,
        rustix::fs::RenameFlags::NOREPLACE,
    )?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
