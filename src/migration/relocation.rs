//! Idempotent same-filesystem moves, bound to the preflight directory identity.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
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
    move_bound(source, destination, expected.device, |path| {
        ensure!(
            Identity::read(path)? == *expected,
            "directory identity changed after preflight"
        );
        Ok(())
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    digest: String,
}

impl FileIdentity {
    pub fn read(path: &Path) -> Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "migration entry must be a regular file");
        let mut hash = Sha256::new();
        let mut bytes = [0_u8; 65536];
        loop {
            let count = file.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            hash.update(&bytes[..count]);
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: metadata.mode(),
            digest: format!("{:x}", hash.finalize()),
        })
    }
}

pub fn move_file(source: &Path, destination: &Path, expected: &FileIdentity) -> Result<()> {
    move_bound(source, destination, expected.device, |path| {
        ensure!(
            FileIdentity::read(path)? == *expected,
            "file identity changed after staging"
        );
        Ok(())
    })
}

fn move_bound(
    source: &Path,
    destination: &Path,
    device: u64,
    verify: impl Fn(&Path) -> Result<()>,
) -> Result<()> {
    ensure!(
        source.is_absolute() && destination.is_absolute() && source != destination,
        "invalid migration directory paths"
    );
    let from_parent = source.parent().context("source has no parent")?;
    let to_parent = destination.parent().context("destination has no parent")?;
    ensure!(
        fs::metadata(to_parent)?.dev() == device,
        "migration cannot copy across filesystems"
    );
    let source_present = match fs::symlink_metadata(source) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if source_present {
        verify(source)?;
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            source,
            rustix::fs::CWD,
            destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )?;
    } else {
        verify(destination)?;
    }
    fs::File::open(from_parent)?.sync_all()?;
    fs::File::open(to_parent)?.sync_all()?;
    Ok(())
}
