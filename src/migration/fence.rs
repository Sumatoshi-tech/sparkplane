//! Persistent reboot guards; a volatile permit is created only after fencing.
use anyhow::{Result, ensure};
use std::{fs, path::Path};

const GUARD: &[u8] = b"[Unit]\nConditionPathExists=/run/sparkplane-migration-permit\n";
const STARTUP: &[u8] = b"[Service]\nTimeoutStartSec=1900s\n";
const UNITS: [&str; 4] = [
    "sy-spark-agent",
    "sy-spark-executor",
    "sparkplane-agent",
    "sparkplane-executor",
];

fn guards(root: &Path) -> impl Iterator<Item = (std::path::PathBuf, &'static [u8])> + '_ {
    UNITS.into_iter().flat_map(|unit| {
        let dir = root.join(format!("etc/systemd/system/{unit}.service.d"));
        std::iter::once((dir.join("90-sparkplane-migration.conf"), GUARD)).chain(
            unit.ends_with("-agent")
                .then(|| (dir.join("91-sparkplane-migration-startup.conf"), STARTUP)),
        )
    })
}

fn check(path: &Path, expected: &[u8]) -> Result<bool> {
    ensure!(
        !path.parent().expect("fixed guard parent").is_symlink(),
        "unit drop-in directory must not be a symlink"
    );
    ensure!(!path.is_symlink(), "migration guard must not be a symlink");
    if !path.try_exists()? {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file()
            && metadata.len() == expected.len() as u64
            && fs::read(path)? == expected,
        "migration guard conflicts with an existing file"
    );
    Ok(true)
}

pub fn install_guards(root: &Path) -> Result<()> {
    // Check the entire set before touching even the first unit.
    for (path, expected) in guards(root) {
        check(&path, expected)?;
    }
    for (path, expected) in guards(root) {
        let parent = path.parent().expect("fixed guard parent");
        ensure!(
            !parent.is_symlink(),
            "unit drop-in directory must not be a symlink"
        );
        fs::create_dir_all(parent)?;
        if !check(&path, expected)? {
            super::publication::write_private(&path, expected)?;
        }
        fs::File::open(parent)?.sync_all()?;
        fs::File::open(parent.parent().expect("unit root"))?.sync_all()?;
    }
    Ok(())
}

pub fn remove_guards(root: &Path) -> Result<()> {
    for (path, expected) in guards(root) {
        check(&path, expected)?;
    }
    for (path, expected) in guards(root) {
        if check(&path, expected)? {
            fs::remove_file(&path)?;
            fs::File::open(path.parent().expect("fixed guard parent"))?.sync_all()?;
        }
    }
    Ok(())
}
