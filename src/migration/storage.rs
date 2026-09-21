//! Concrete database/cache cutover; originals remain recoverable until commit.
use super::relocation::{FileIdentity, Identity, move_file, relocate};
use crate::spark::engine::EnginePolicy;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, os::unix::fs::MetadataExt, path::Path};

const DATABASE_FILES: [&str; 3] = ["state.sqlite3", "state.sqlite3-wal", "state.sqlite3-shm"];

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    data: Identity,
    original: Vec<(String, FileIdentity)>,
    candidate: FileIdentity,
    caches: Vec<(String, String, Identity)>,
    #[serde(default)]
    emergency: super::emergency::Journal,
}

fn cache_key(key: &str) -> bool {
    key.strip_prefix("sha256-")
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

impl Storage {
    pub fn prepare(
        root: &Path,
        work: &Path,
        engines: &[EnginePolicy],
        cache_keys: &[(String, String)],
    ) -> Result<Self> {
        let data = root.join("var/lib/sy-spark");
        let metadata = fs::symlink_metadata(&data)?;
        ensure!(
            !root.join("var/lib/sparkplane").try_exists()?
                && !root.join("var/lib/sparkplane").is_symlink(),
            "data destination already exists"
        );
        ensure!(
            metadata.dev() == fs::metadata(work)?.dev(),
            "database recovery must remain on the same filesystem"
        );
        for directory in ["original-db", "failed-db"] {
            fs::create_dir(work.join(directory))?;
        }
        let candidate = work.join("candidate.sqlite3");
        super::stage_database(&data.join(DATABASE_FILES[0]), &candidate, engines)?;
        // Preserve numeric identity; never recursively chown the model tree.
        rustix::fs::chown(
            &candidate,
            Some(rustix::fs::Uid::from_raw(metadata.uid())),
            Some(rustix::fs::Gid::from_raw(metadata.gid())),
        )?;
        let original = DATABASE_FILES
            .into_iter()
            .filter_map(|name| match fs::symlink_metadata(data.join(name)) {
                Ok(_) => Some(FileIdentity::read(&data.join(name)).map(|id| (name.into(), id))),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => Some(Err(error.into())),
            })
            .collect::<Result<Vec<_>>>()?;
        let mut caches = Vec::new();
        for (old, new) in cache_keys {
            ensure!(
                cache_key(old) && cache_key(new) && old != new,
                "invalid compile-cache move"
            );
            if caches.iter().any(|(seen, _, _)| seen == old) {
                continue;
            }
            let parent = data.join("compile-cache");
            ensure!(
                !parent.join(new).try_exists()? && !parent.join(new).is_symlink(),
                "compile-cache destination already exists"
            );
            if parent.join(old).try_exists()? {
                caches.push((old.clone(), new.clone(), Identity::read(&parent.join(old))?));
            }
        }
        Ok(Self {
            emergency: super::emergency::Journal::prepare(&data, work)?,
            data: Identity::read(&data)?,
            original,
            candidate: FileIdentity::read(&candidate)?,
            caches,
        })
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.original
                .iter()
                .all(|(name, _)| DATABASE_FILES.contains(&name.as_str()))
                && self
                    .original
                    .iter()
                    .filter(|(name, _)| name == DATABASE_FILES[0])
                    .count()
                    == 1,
            "invalid recovery database inventory"
        );
        ensure!(
            self.caches
                .iter()
                .all(|(old, new, _)| cache_key(old) && cache_key(new) && old != new),
            "invalid recovery cache inventory"
        );
        Ok(())
    }

    pub fn activate(&self, root: &Path, work: &Path) -> Result<()> {
        self.validate()?;
        let old = root.join("var/lib/sy-spark");
        let new = root.join("var/lib/sparkplane");
        let data = if old.try_exists()? { &old } else { &new };
        ensure!(Identity::read(data)? == self.data, "data identity changed");
        for (name, id) in &self.original {
            let backup = work.join("original-db").join(name);
            if backup.try_exists()? {
                ensure!(
                    FileIdentity::read(&backup)? == *id,
                    "original database backup changed"
                );
            } else {
                move_file(&data.join(name), &backup, id)?;
            }
        }
        move_file(
            &work.join("candidate.sqlite3"),
            &data.join(DATABASE_FILES[0]),
            &self.candidate,
        )?;
        relocate(&old, &new, &self.data)?;
        for (from, to, id) in &self.caches {
            let parent = new.join("compile-cache");
            relocate(&parent.join(from), &parent.join(to), id)?;
        }
        self.emergency.activate(&new, work)?;
        Ok(())
    }

    pub fn restore(&self, root: &Path, work: &Path) -> Result<()> {
        self.validate()?;
        let old = root.join("var/lib/sy-spark");
        let new = root.join("var/lib/sparkplane");
        self.emergency
            .restore(if new.try_exists()? { &new } else { &old }, work)?;
        if new.try_exists()? {
            for (from, to, id) in self.caches.iter().rev() {
                let parent = new.join("compile-cache");
                if parent.join(to).try_exists()? {
                    relocate(&parent.join(to), &parent.join(from), id)?;
                } else {
                    ensure!(
                        Identity::read(&parent.join(from))? == *id,
                        "cache recovery identity changed"
                    );
                }
            }
            relocate(&new, &old, &self.data)?;
        }
        ensure!(
            Identity::read(&old)? == self.data,
            "data recovery identity changed"
        );
        for name in DATABASE_FILES {
            let path = old.join(name);
            let original = self.original.iter().find(|(entry, _)| entry == name);
            let backup = work.join("original-db").join(name);
            if let Some((_, identity)) = original
                && !backup.try_exists()?
            {
                ensure!(
                    FileIdentity::read(&path)? == *identity,
                    "original database is missing"
                );
                continue;
            }
            if path.try_exists()? {
                move_file(
                    &path,
                    &work.join("failed-db").join(name),
                    &FileIdentity::read(&path)?,
                )?;
            }
            if let Some((_, identity)) = original {
                move_file(&backup, &path, identity)?;
            }
        }
        Ok(())
    }
}
