//! Digest-based ownership for the two generated coding-client files.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};

const FILES: [&str; 2] = [
    "sparkplane-launch.config.toml",
    "sparkplane-launch-models.json",
];
const RECEIPT: &str = "sparkplane-launch.ownership.json";
const SCHEMA: &str = "sparkplane.generated-files/v1";
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: String,
    files: BTreeMap<String, Vec<String>>,
}

fn read(home: &Path, name: &str) -> Result<Option<Vec<u8>>> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(home.join(name))
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.mode() & 0o022 == 0
            && metadata.len() <= MAX_BYTES,
        "generated file has unsafe ownership, type or size"
    );
    let mut bytes = Vec::new();
    (&mut file).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "generated file exceeds size limit"
    );
    Ok(Some(bytes))
}

fn atomic(home: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let temporary = home.join(format!(".sparkplane-generated-{}", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temporary, home.join(name))?;
    fs::File::open(home)?.sync_all()?;
    Ok(())
}

fn receipt(home: &Path) -> Result<Receipt> {
    let receipt: Receipt = match read(home, RECEIPT)? {
        Some(bytes) => serde_json::from_slice(&bytes)?,
        None => Receipt {
            schema: SCHEMA.into(),
            files: BTreeMap::new(),
        },
    };
    ensure!(
        receipt.schema == SCHEMA
            && receipt
                .files
                .iter()
                .all(|(name, hashes)| FILES.contains(&name.as_str())
                    && !hashes.is_empty()
                    && hashes.len() <= 2
                    && hashes.iter().all(
                        |hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                    )),
        "invalid generated-file ownership receipt"
    );
    Ok(receipt)
}

fn lock(home: &Path) -> Result<fs::File> {
    let metadata = fs::metadata(home)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.mode() & 0o022 == 0,
        "generated-file directory must be owned and not shared-writable"
    );
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(home.join(".sparkplane-launch.lock"))?;
    file.try_lock()
        .context("another launch is changing generated files")?;
    Ok(file)
}

pub fn publish(home: &Path, profile: &[u8], catalog: &[u8]) -> Result<()> {
    ensure!(
        profile.len() as u64 <= MAX_BYTES && catalog.len() as u64 <= MAX_BYTES,
        "generated payload exceeds size limit"
    );
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(home)?;
    let _lock = lock(home)?;
    let mut pending = receipt(home)?;
    let mut committed = Receipt {
        schema: SCHEMA.into(),
        files: BTreeMap::new(),
    };
    for (name, bytes) in FILES.into_iter().zip([profile, catalog]) {
        let hash = format!("{:x}", Sha256::digest(bytes));
        let mut owned = vec![hash.clone()];
        if let Some(current) = read(home, name)? {
            let current_hash = format!("{:x}", Sha256::digest(&current));
            ensure!(
                current == bytes
                    || pending
                        .files
                        .get(name)
                        .is_some_and(|hashes| hashes.contains(&current_hash)),
                "refusing to overwrite modified or unowned generated file {name}"
            );
            if current_hash != hash {
                owned.push(current_hash);
            }
        }
        pending.files.insert(name.into(), owned);
        committed.files.insert(name.into(), vec![hash]);
    }
    // Both old and new hashes are owned during a recoverable interrupted publication.
    atomic(home, RECEIPT, &serde_json::to_vec(&pending)?)?;
    for (name, bytes) in FILES.into_iter().zip([profile, catalog]) {
        atomic(home, name, bytes)?;
    }
    atomic(home, RECEIPT, &serde_json::to_vec(&committed)?)
}

pub fn remove(home: &Path) -> Result<()> {
    let _lock = lock(home)?;
    let receipt = receipt(home)?;
    let mut present = Vec::new();
    for name in FILES {
        if let Some(bytes) = read(home, name)? {
            let hash = format!("{:x}", Sha256::digest(bytes));
            ensure!(
                receipt
                    .files
                    .get(name)
                    .is_some_and(|hashes| hashes.contains(&hash)),
                "refusing to remove modified or unowned generated file {name}"
            );
            present.push(name);
        }
    }
    for name in present {
        fs::remove_file(home.join(name))?;
    }
    atomic(
        home,
        RECEIPT,
        &serde_json::to_vec(&Receipt {
            schema: SCHEMA.into(),
            files: BTreeMap::new(),
        })?,
    )
}
