//! Exact legacy container ownership; names and label selectors never grant cleanup authority.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyContainer {
    pub container_id: String,
    pub instance_id: String,
    pub generation: u64,
    pub image_digest: String,
    pub engine_id: String,
    pub engine_fingerprint: String,
    pub artifact_fingerprint: String,
    pub model_repository: String,
    pub model_commit: String,
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl LegacyContainer {
    pub fn labels(&self) -> BTreeMap<&'static str, String> {
        BTreeMap::from([
            ("io.sy.spark.managed", "true".into()),
            ("io.sy.spark.role", "engine".into()),
            ("io.sy.spark.instance", self.instance_id.clone()),
            ("io.sy.spark.generation", self.generation.to_string()),
            ("io.sy.spark.engine", self.engine_id.clone()),
            (
                "io.sy.spark.engine_fingerprint",
                self.engine_fingerprint.clone(),
            ),
            (
                "io.sy.spark.artifact_fingerprint",
                self.artifact_fingerprint.clone(),
            ),
            (
                "io.sy.spark.model_repository",
                self.model_repository.clone(),
            ),
            ("io.sy.spark.model_commit", self.model_commit.clone()),
        ])
    }

    pub fn verify(&self, inspect: &serde_json::Value) -> Result<()> {
        ensure!(
            hex(&self.container_id, 64)
                && self
                    .instance_id
                    .strip_prefix("i_")
                    .is_some_and(|s| hex(s, 32))
                && self.generation > 0,
            "invalid exact container identity"
        );
        ensure!(
            !self.engine_id.is_empty()
                && self.engine_id.len() <= 128
                && self
                    .engine_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid engine identity"
        );
        ensure!(
            [
                &self.image_digest,
                &self.engine_fingerprint,
                &self.artifact_fingerprint
            ]
            .iter()
            .all(|s| s.strip_prefix("sha256:").is_some_and(|s| hex(s, 64))),
            "invalid container content identity"
        );
        crate::spark::model::Repository::parse(&self.model_repository)?;
        crate::spark::model::CommitSha::parse(&self.model_commit)?;
        ensure!(
            inspect.get("Id").and_then(serde_json::Value::as_str) == Some(&self.container_id)
                && inspect.get("Image").and_then(serde_json::Value::as_str)
                    == Some(&self.image_digest)
                && inspect.get("Name").and_then(serde_json::Value::as_str)
                    == Some(&format!(
                        "/sy-spark-{}-g{}",
                        self.instance_id, self.generation
                    )),
            "container identity differs from preflight"
        );
        let labels = inspect.pointer("/Config/Labels");
        ensure!(
            self.labels().iter().all(|(key, value)| labels
                .and_then(|l| l.get(key))
                .and_then(serde_json::Value::as_str)
                == Some(value)),
            "container ownership differs from preflight"
        );
        Ok(())
    }
}
