//! Idempotent same-filesystem moves, bound to the preflight directory identity.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, os::unix::fs::MetadataExt, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
}

impl Identity {
    pub fn read(path: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_dir(),
            "migration source must be a real directory"
        );
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
            gid: metadata.gid(),
        })
    }
}

/// Retrying after rename but before journal fsync succeeds only for the exact inode.
pub fn relocate(source: &Path, destination: &Path, expected: &Identity) -> Result<()> {
    ensure!(
        source.is_absolute() && destination.is_absolute() && source != destination,
        "invalid migration directory paths"
    );
    let from_parent = source.parent().context("source has no parent")?;
    let to_parent = destination.parent().context("destination has no parent")?;
    ensure!(
        fs::metadata(to_parent)?.dev() == expected.device,
        "migration cannot copy across filesystems"
    );
    let source_present = match fs::symlink_metadata(source) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if source_present {
        ensure!(
            Identity::read(source)? == *expected,
            "source identity changed after preflight"
        );
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            source,
            rustix::fs::CWD,
            destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )?;
    } else {
        ensure!(
            Identity::read(destination)? == *expected,
            "interrupted move destination identity mismatch"
        );
    }
    fs::File::open(from_parent)?.sync_all()?;
    fs::File::open(to_parent)?.sync_all()?;
    Ok(())
}
