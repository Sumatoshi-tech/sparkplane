//! Preserve executor emergency evidence while converting its typed replay journal.
use crate::spark::resources::{EmergencyRecord, read_emergency_records};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags};
use std::{fs, path::Path};

const JOURNAL: &str = "executor/emergency.jsonl";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    original: Option<super::relocation::FileIdentity>,
    candidate: Option<super::relocation::FileIdentity>,
}

impl Journal {
    pub fn prepare(data: &Path, work: &Path) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let Some(bytes) = validate(data)? else {
            return Ok(Self::default());
        };
        let source = data.join(JOURNAL);
        let original = super::relocation::FileIdentity::read(&source)?;
        let candidate = work.join("candidate-emergency.jsonl");
        super::publication::write_private(&candidate, &bytes)?;
        let metadata = fs::symlink_metadata(source)?;
        rustix::fs::chown(
            &candidate,
            Some(rustix::fs::Uid::from_raw(metadata.uid())),
            Some(rustix::fs::Gid::from_raw(metadata.gid())),
        )?;
        fs::File::open(&candidate)?.sync_all()?;
        Ok(Self {
            original: Some(original),
            candidate: Some(super::relocation::FileIdentity::read(&candidate)?),
        })
    }

    pub fn activate(&self, data: &Path, work: &Path) -> Result<()> {
        use super::relocation::{FileIdentity, move_file};
        ensure!(
            self.original.is_some() == self.candidate.is_some(),
            "invalid emergency snapshot"
        );
        if let (Some(original), Some(candidate)) = (&self.original, &self.candidate) {
            let backup = work.join("original-emergency.jsonl");
            if backup.try_exists()? {
                ensure!(
                    FileIdentity::read(&backup)? == *original,
                    "emergency snapshot changed"
                );
            } else {
                move_file(&data.join(JOURNAL), &backup, original)?;
            }
            move_file(
                &work.join("candidate-emergency.jsonl"),
                &data.join(JOURNAL),
                candidate,
            )?;
        }
        Ok(())
    }

    pub fn restore(&self, data: &Path, work: &Path) -> Result<()> {
        use super::relocation::{FileIdentity, move_file};
        let backup = work.join("original-emergency.jsonl");
        let current = data.join(JOURNAL);
        if let Some(original) = &self.original
            && !backup.try_exists()?
        {
            ensure!(
                FileIdentity::read(&current)? == *original,
                "original emergency journal is missing"
            );
            return Ok(());
        }
        if current.try_exists()? || current.is_symlink() {
            move_file(
                &current,
                &work.join("failed-emergency.jsonl"),
                &FileIdentity::read(&current)?,
            )?;
        }
        if let Some(original) = &self.original {
            move_file(&backup, &current, original)?;
        }
        Ok(())
    }
}

pub fn validate(data: &Path) -> Result<Option<Vec<u8>>> {
    ensure!(
        !data.join("executor").is_symlink(),
        "executor journal directory is a symlink"
    );
    let metadata = match fs::symlink_metadata(data.join(JOURNAL)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        result => result?,
    };
    ensure!(
        metadata.is_file(),
        "legacy emergency journal must be a regular file"
    );
    let records = read_emergency_records(&data.join(JOURNAL))
        .map_err(|_| anyhow::anyhow!("legacy emergency journal is invalid"))?;
    let db =
        Connection::open_with_flags(data.join("state.sqlite3"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut output = Vec::new();
    for mut record in records {
        ensure!(
            record.schema == "sy.spark.emergency-record/v1"
                && record.decision.schema == "sy.spark.emergency-decision/v1"
                && record.event_id.parse::<ulid::Ulid>().is_ok(),
            "unsupported legacy emergency record"
        );
        let evidence: String = db.query_row(
            "SELECT evidence_json FROM emergency_records WHERE event_id=?1",
            [&record.event_id],
            |row| row.get(0),
        )?;
        ensure!(
            serde_json::from_str::<EmergencyRecord>(&evidence)? == record,
            "legacy emergency journal differs from imported evidence"
        );
        record.schema = "sparkplane.emergency-record/v1".into();
        record.decision.schema = "sparkplane.emergency-decision/v1".into();
        serde_json::to_writer(&mut output, &record)?;
        output.push(b'\n');
    }
    Ok(Some(output))
}
