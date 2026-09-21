//! Installed-authority approval for a bounded, post-restoration recovery handoff.
use super::{appliance::Plan, container::LegacyContainer};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    pub schema: String,
    pub host: String,
    pub record_sha256: String,
    pub executable_sha256: String,
    pub expires_at_unix_seconds: u64,
    pub replacements: Vec<LegacyContainer>,
}

impl Approval {
    pub fn apply(&self, plan: &mut Plan, inventory: &[serde_json::Value]) -> Result<()> {
        ensure!(
            self.host == plan.host
                && self.replacements.len() == plan.active.len()
                && inventory.len() == plan.active.len(),
            "recovery instance set changed"
        );
        let mut verified = Vec::new();
        for active in &plan.active {
            let replacement = self
                .replacements
                .iter()
                .find(|r| r.instance_id == active.before.id)
                .context("recovery replacement missing")?;
            let mut expected = active.container.clone();
            ensure!(
                replacement.generation > expected.generation,
                "replacement generation must advance"
            );
            expected.generation = replacement.generation;
            expected.container_id.clone_from(&replacement.container_id);
            ensure!(
                replacement == &expected,
                "recovery changes immutable engine identity"
            );
            let inspect = inventory
                .iter()
                .find(|v| v["Id"] == replacement.container_id)
                .context("approved replacement container missing")?;
            replacement.verify(inspect)?;
            ensure!(
                inspect["State"]["Running"] == true,
                "replacement is not running"
            );
            super::container::verify_restart_policy(inspect)?;
            verified.push(replacement.clone());
        }
        for (active, replacement) in plan.active.iter_mut().zip(verified) {
            active.before.generation = replacement.generation;
            active.container = replacement;
        }
        Ok(())
    }
    pub fn verify(
        bytes: &[u8],
        signature: &str,
        key: &str,
        identities: [&str; 3],
        now: u64,
    ) -> Result<Self> {
        ensure!(bytes.len() <= 65536, "recovery approval exceeds size limit");
        minisign_verify::PublicKey::from_base64(key)?.verify(
            bytes,
            &minisign_verify::Signature::decode(signature)?,
            false,
        )?;
        let approval: Self = serde_json::from_slice(bytes)?;
        ensure!(
            approval.schema == "sparkplane.recovery-approval/v1"
                && now < approval.expires_at_unix_seconds,
            "unsupported or expired recovery approval"
        );
        ensure!(
            identities
                .iter()
                .all(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                && identities
                    == [
                        &approval.host,
                        &approval.record_sha256,
                        &approval.executable_sha256
                    ],
            "recovery approval identity mismatch"
        );
        Ok(approval)
    }
}
