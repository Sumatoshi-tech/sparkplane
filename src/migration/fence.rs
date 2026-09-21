//! Persistent reboot guards; a volatile permit is created only after fencing.
use anyhow::{Result, ensure};
use std::{fs, path::Path};

const GUARD: &[u8] = b"[Unit]\nConditionPathExists=/run/sparkplane-migration-permit\n";
const UNITS: [&str; 4] = [
    "sy-spark-agent",
    "sy-spark-executor",
    "sparkplane-agent",
    "sparkplane-executor",
];

fn guards(root: &Path) -> impl Iterator<Item = std::path::PathBuf> + '_ {
    UNITS.into_iter().map(|unit| {
        root.join(format!(
            "etc/systemd/system/{unit}.service.d/90-sparkplane-migration.conf"
        ))
    })
}

fn check(path: &Path) -> Result<bool> {
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
        metadata.is_file() && metadata.len() == GUARD.len() as u64 && fs::read(path)? == GUARD,
        "migration guard conflicts with an existing file"
    );
    Ok(true)
}

pub fn install_guards(root: &Path) -> Result<()> {
    // Check the entire set before touching even the first unit.
    for path in guards(root) {
        check(&path)?;
    }
    for path in guards(root) {
        let parent = path.parent().expect("fixed guard parent");
        ensure!(
            !parent.is_symlink(),
            "unit drop-in directory must not be a symlink"
        );
        fs::create_dir_all(parent)?;
        if !check(&path)? {
            super::publication::write_private(&path, GUARD)?;
        }
        fs::File::open(parent)?.sync_all()?;
        fs::File::open(parent.parent().expect("unit root"))?.sync_all()?;
    }
    Ok(())
}

pub fn remove_guards(root: &Path) -> Result<()> {
    for path in guards(root) {
        check(&path)?;
    }
    for path in guards(root) {
        if check(&path)? {
            fs::remove_file(&path)?;
            fs::File::open(path.parent().expect("fixed guard parent"))?.sync_all()?;
        }
    }
    Ok(())
}
