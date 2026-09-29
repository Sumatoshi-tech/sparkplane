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
const PROFILE: &str = FILES[0];
const RECEIPT: &str = "sparkplane-launch.ownership.json";
const SCHEMA: &str = "sparkplane.generated-files/v1";
const MAX_BYTES: u64 = 1024 * 1024;
/// Assignments written by `codex_client_config`. Codex persists other settings,
/// such as `approvals_reviewer`, into the active profile; those lines are not
/// a conflict. Edits to these keys still are.
const MANAGED_KEYS: [&str; 9] = [
    "model",
    "model_provider",
    "web_search",
    "name",
    "base_url",
    "env_key",
    "wire_api",
    "supports_standalone_web_search",
    "supports_websockets",
];

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
            let hashes = pending.files.get(name).map(Vec::as_slice).unwrap_or(&[]);
            ensure!(
                current == bytes || still_owned(name, &current, hashes),
                "{}",
                conflict(home, name, "overwrite")
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
            let hashes = receipt.files.get(name).map(Vec::as_slice).unwrap_or(&[]);
            ensure!(
                still_owned(name, &bytes, hashes),
                "{}",
                conflict(home, name, "remove")
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

fn still_owned(name: &str, bytes: &[u8], hashes: &[String]) -> bool {
    if owned_digest(bytes, hashes) {
        return true;
    }
    // Codex rewrites the live profile with settings Sparkplane does not generate.
    // The managed lines must still be exactly the bytes Sparkplane published.
    name == PROFILE && owned_digest(&without_foreign_assignments(bytes), hashes)
}

fn owned_digest(bytes: &[u8], hashes: &[String]) -> bool {
    let digest = format!("{:x}", Sha256::digest(bytes));
    hashes.iter().any(|hash| hash == &digest)
}

fn without_foreign_assignments(bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    let mut kept = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if !foreign_assignment(line) {
            kept.push_str(line);
        }
    }
    kept.into_bytes()
}

fn foreign_assignment(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(['#', '[']) {
        return false;
    }
    let Some((key, _)) = trimmed.split_once('=') else {
        return false;
    };
    let key = key.trim();
    !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !MANAGED_KEYS.contains(&key)
}

fn conflict(home: &Path, name: &str, action: &str) -> String {
    format!(
        "refusing to {action} {}: it no longer matches the Sparkplane ownership receipt. Move it aside and retry",
        home.join(name).display()
    )
}
