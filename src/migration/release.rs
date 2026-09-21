//! Hold verified release bytes in memory to close staging-path replacement races.
use crate::spark::install::{self, ReleaseBundle, ReleaseEngine};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

pub struct Release {
    pub manifest: Vec<u8>,
    executable: Vec<u8>,
    models: Vec<u8>,
    pub engines: BTreeMap<String, Vec<u8>>,
    signature: String,
    public_key: String,
    pub executable_sha256: String,
}

pub fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "migration input must be a regular file"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "migration input exceeds size limit"
    );
    Ok(bytes)
}

impl Release {
    pub fn verify_model_catalog(&self, legacy: &str) -> Result<()> {
        let mut old: toml::Value = toml::from_str(legacy)?;
        ensure!(
            old.get("schema").and_then(toml::Value::as_str) == Some("sy.spark.models/v2"),
            "unsupported source model catalog"
        );
        old["schema"] = "sparkplane.models/v2".into();
        let current: toml::Value = toml::from_str(std::str::from_utf8(&self.models)?)?;
        ensure!(
            old == current,
            "migration changes model catalog beyond its namespace"
        );
        Ok(())
    }

    pub fn load(directory: &Path, public_key: &str) -> Result<Self> {
        let manifest = read(&directory.join("SHA256SUMS"), 65536)?;
        let signature = String::from_utf8(read(&directory.join("SHA256SUMS.minisig"), 4096)?)?;
        minisign_verify::PublicKey::from_base64(public_key)?.verify(
            &manifest,
            &minisign_verify::Signature::decode(&signature)?,
            false,
        )?;
        let mut engines = BTreeMap::new();
        for line in std::str::from_utf8(&manifest)?.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            ensure!(fields.len() == 2, "invalid release inventory entry");
            let path = fields[1].trim_start_matches('*');
            if let Some(name) = path.strip_prefix("configs/sparkplane/engines/") {
                ensure!(
                    !name.contains('/')
                        && name.ends_with(".toml")
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
                    "invalid signed engine path"
                );
                ensure!(
                    engines.len() < 64 && !engines.contains_key(path),
                    "duplicate or excessive engine inventory"
                );
                engines.insert(path.into(), read(&directory.join(path), 1024 * 1024)?);
            }
        }
        let executable = read(&directory.join("sparkplane-aarch64"), 512 * 1024 * 1024)?;
        let release = Self {
            executable_sha256: format!("{:x}", Sha256::digest(&executable)),
            executable,
            manifest,
            signature,
            public_key: public_key.into(),
            engines,
            models: read(
                &directory.join("configs/sparkplane/models.toml"),
                1024 * 1024,
            )?,
        };
        release.with_bundle("127.0.0.1", "localhost", install::validate_release)?;
        Ok(release)
    }

    fn with_bundle<T>(
        &self,
        address: &str,
        hostname: &str,
        action: impl FnOnce(&ReleaseBundle<'_>) -> Result<T, install::InstallError>,
    ) -> Result<T> {
        let engines: Vec<_> = self
            .engines
            .iter()
            .map(|(relative_path, bytes)| ReleaseEngine {
                relative_path,
                bytes,
            })
            .collect();
        Ok(action(&ReleaseBundle {
            version: env!("CARGO_PKG_VERSION"),
            executable: &self.executable,
            executable_sha256: &self.executable_sha256,
            release_manifest: &self.manifest,
            models: &self.models,
            engines: &engines,
            public_key_base64: &self.public_key,
            signature: &self.signature,
            listen_address: address,
            hostname,
            active_lsm: "apparmor:enforce",
        })?)
    }

    pub fn stage(&self, root: &Path, address: &str, hostname: &str) -> Result<()> {
        ensure!(
            root != Path::new("/"),
            "migration release staging must never activate the live installer"
        );
        self.with_bundle(address, hostname, |bundle| {
            install::install_release(root, bundle)
        })?;
        Ok(())
    }
}
