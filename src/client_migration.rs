//! Atomic import of only legacy Spark client files; never traverses sy settings.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

#[derive(serde::Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub files: usize,
    pub dry_run: bool,
    pub source_preserved: bool,
}

pub fn migrate(source: &Path, destination: &Path, dry_run: bool) -> Result<Report> {
    ensure!(
        source.is_absolute() && destination.is_absolute(),
        "migration paths must be absolute"
    );
    ensure!(
        !destination.try_exists()? && !destination.is_symlink(),
        "client destination already exists; resolve the conflict first"
    );
    ensure!(
        !fs::symlink_metadata(source)?.file_type().is_symlink(),
        "legacy configuration must not be a symlink"
    );
    let source = source.canonicalize()?;
    for relative in [
        "spark-launch.toml",
        "spark",
        "credentials",
        "credentials/spark",
    ] {
        ensure!(
            !source.join(relative).is_symlink(),
            "legacy client contains a symlink"
        );
    }
    let parent = destination
        .parent()
        .context("client destination has no parent")?
        .canonicalize()?;
    ensure!(
        !parent.starts_with(&source),
        "client destination must not be inside legacy configuration"
    );
    let mut paths = vec![PathBuf::from("spark.toml")];
    for relative in ["spark-launch.toml", "spark", "credentials/spark"] {
        let directory = source.join(relative);
        if !directory.try_exists()? {
            continue;
        }
        for entry in walkdir::WalkDir::new(&directory).follow_links(false) {
            let entry = entry?;
            ensure!(
                !entry.file_type().is_symlink(),
                "legacy client contains a symlink"
            );
            if entry.file_type().is_file() {
                paths.push(entry.path().strip_prefix(&source)?.to_path_buf());
            }
        }
    }
    ensure!(
        paths.len() <= 1024,
        "legacy client inventory exceeds the file limit"
    );
    let mut payload = Vec::new();
    let mut total = 0_u64;
    for relative in paths {
        let path = source.join(&relative);
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "legacy client entry must be a regular file"
        );
        total = total
            .checked_add(metadata.len())
            .context("client inventory overflow")?;
        ensure!(
            total <= 16 * 1024 * 1024,
            "legacy client inventory exceeds the byte limit"
        );
        let mut bytes = fs::read(&path)?;
        if relative == Path::new("spark-launch.toml") {
            bytes = crate::spark::launch::migrate_legacy_state(std::str::from_utf8(&bytes)?)
                .map_err(anyhow::Error::msg)?
                .into_bytes();
        }
        if relative == Path::new("spark.toml") {
            toml::from_str::<toml::Value>(std::str::from_utf8(&bytes)?)?;
        }
        payload.push((relative, bytes));
    }
    let report = Report {
        schema: "sparkplane.client-migration/v1",
        files: payload.len(),
        dry_run,
        source_preserved: true,
    };
    if dry_run {
        return Ok(report);
    }
    let stage = parent.join(format!(".sparkplane-client-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&stage)?;
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o700))?;
    for (relative, bytes) in payload {
        let path = stage.join(relative);
        let directory = path.parent().context("client entry has no parent")?;
        fs::create_dir_all(directory)?;
        fs::write(&path, bytes)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        fs::File::open(&path)?.sync_all()?;
        fs::File::open(directory)?.sync_all()?;
    }
    fs::File::open(&stage)?.sync_all()?;
    // renameat2 NOREPLACE closes the destination-conflict race without overwriting user files.
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        &stage,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )?;
    fs::File::open(parent)?.sync_all()?;
    Ok(report)
}
