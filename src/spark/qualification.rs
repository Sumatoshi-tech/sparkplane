//! Signed, typed GPU qualification contracts for the Spark plane.

#[cfg(feature = "appliance")]
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
#[cfg(feature = "appliance")]
use sha2::{Digest, Sha256};
#[cfg(feature = "appliance")]
use std::io::{Cursor, Read};

#[cfg(feature = "appliance")]
pub const QUALIFICATION_MANIFEST_SCHEMA: &str = "sparkplane.qualification-manifest/v1";
#[cfg(feature = "appliance")]
pub const QUALIFICATION_PLAN_SCHEMA: &str = "sparkplane.qualification-plan/v1";
#[cfg(feature = "appliance")]
pub const QUALIFICATION_RESULT_SCHEMA: &str = "sparkplane.qualification-result/v1";
#[cfg(feature = "appliance")]
pub const RELEASE_PUBLIC_KEY_PATH: &str = "/opt/sparkplane/current/minisign.pub";
#[cfg(feature = "appliance")]
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
#[cfg(feature = "appliance")]
const MAX_SIGNATURE_BYTES: usize = 4 * 1024;
#[cfg(feature = "appliance")]
const MAX_MEMORY_BYTES: u64 = 64 * 1024 * 1024 * 1024;
#[cfg(feature = "appliance")]
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
#[cfg(feature = "appliance")]
const MAX_PIDS: u32 = 256;
#[cfg(feature = "appliance")]
const MAX_CPU_CORES: u16 = 20;
#[cfg(feature = "appliance")]
const MAX_TIMEOUT_SECONDS: u64 = 1_800;
#[cfg(feature = "appliance")]
const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;
#[cfg(feature = "appliance")]
const MAX_ENCODED_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Exact signed material accepted by the qualification endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "appliance", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct QualificationSubmission {
    pub manifest_base64: String,
    pub signature: String,
    pub dry_run: bool,
}

/// Finite runner operation. No executable or arbitrary argument is serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "appliance", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum QualificationOperation {
    SparkFlashAdaptiveDecodingV1,
    SparkFlashQsaV1,
    SparkFlashMoeV1,
    SparkFlashKernelCatalogV1,
    SparkFlashCudaLifecycleV1,
    SparkFlashMemoryAdmissionV1,
    SparkFlashGraphCatalogV1,
    SparkFlashSessionV1,
    SparkFlashModelPrefixV1,
}

#[cfg(feature = "appliance")]
impl QualificationOperation {
    /// Fixed JIT environment owned by qualification policy, not engine configuration.
    pub const fn jit_environment(self) -> &'static [&'static str] {
        match self {
            Self::SparkFlashQsaV1 | Self::SparkFlashMoeV1 => {
                &["HOME=/tmp", "TRITON_CACHE_DIR=/tmp/triton"]
            }
            Self::SparkFlashAdaptiveDecodingV1 | Self::SparkFlashKernelCatalogV1 => &[],
            Self::SparkFlashCudaLifecycleV1 | Self::SparkFlashMemoryAdmissionV1 => &[],
            Self::SparkFlashGraphCatalogV1 => &[],
            Self::SparkFlashSessionV1 => &[],
            Self::SparkFlashModelPrefixV1 => &[],
        }
    }

    pub const fn executable(self) -> &'static str {
        match self {
            Self::SparkFlashAdaptiveDecodingV1 => "/opt/sparky/bin/sparky-qualification",
            Self::SparkFlashQsaV1 => "/opt/sparky/bin/sparky-qualification",
            Self::SparkFlashMoeV1 => "/opt/sparky/bin/sparky-qualification",
            Self::SparkFlashKernelCatalogV1 => "/opt/sparky/bin/sparky-qualification",
            Self::SparkFlashCudaLifecycleV1 => "/opt/sparky/bin/sparky-qualification",
            Self::SparkFlashMemoryAdmissionV1 => "/opt/sparky/bin/sparky-qualification",
            Self::SparkFlashGraphCatalogV1 => "/opt/sparky/bin/sparky-qualification",
            Self::SparkFlashSessionV1 => "/opt/sparky/bin/sparky-qualification",
            Self::SparkFlashModelPrefixV1 => "/opt/sparky/bin/sparky-qualification",
        }
    }

    pub const fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::SparkFlashAdaptiveDecodingV1 => &["adaptive-decoding-target", "--json"],
            Self::SparkFlashQsaV1 => &["qsa-target", "--json"],
            Self::SparkFlashMoeV1 => &["moe-target", "--json"],
            Self::SparkFlashKernelCatalogV1 => &["kernel-catalog-target", "--json"],
            Self::SparkFlashCudaLifecycleV1 => &["cuda-lifecycle-target", "--json"],
            Self::SparkFlashMemoryAdmissionV1 => &["memory-admission-target", "--json"],
            Self::SparkFlashGraphCatalogV1 => &["graph-catalog-target", "--json"],
            Self::SparkFlashSessionV1 => &["session-target", "--json"],
            Self::SparkFlashModelPrefixV1 => &["model-prefix-target", "--json"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "appliance", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct QualificationTarget {
    pub architecture: String,
    pub gpu_model: String,
    pub compute_capability: String,
    pub dgx_build: String,
    pub driver_version: String,
    pub toolkit_version: String,
    pub protected_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "appliance", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct QualificationLimits {
    pub memory_bytes: u64,
    pub image_bytes: u64,
    pub pids: u32,
    pub cpu_cores: u16,
    pub timeout_seconds: u64,
    pub output_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "appliance", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
#[cfg(feature = "appliance")]
pub struct QualificationManifest {
    pub schema: String,
    pub job_id: String,
    pub operation: QualificationOperation,
    pub image: String,
    pub target: QualificationTarget,
    pub limits: QualificationLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "appliance", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct QualificationIsolation {
    pub gpu_device: String,
    pub network: String,
    pub read_only_root: bool,
    pub privileged: bool,
    pub capabilities: Vec<String>,
    pub no_new_privileges: bool,
    pub restart: String,
}

impl Default for QualificationIsolation {
    fn default() -> Self {
        Self {
            gpu_device: "0".into(),
            network: "none".into(),
            read_only_root: true,
            privileged: false,
            capabilities: Vec::new(),
            no_new_privileges: true,
            restart: "no".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "appliance", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct QualificationPlanDocument {
    pub schema: String,
    pub admitted: bool,
    pub job_id: String,
    pub operation: QualificationOperation,
    pub manifest_sha256: String,
    pub image: String,
    pub image_digest: String,
    pub target: QualificationTarget,
    pub limits: QualificationLimits,
    pub isolation: QualificationIsolation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<QualificationResourceAdmission>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "appliance", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct QualificationResourceAdmission {
    pub observed_at_unix_ms: u64,
    pub mem_available_bytes: u64,
    pub live_available_after_start_bytes: u64,
    pub required_available_floor_bytes: u64,
    pub disk_available_bytes: u64,
    pub disk_available_after_pull_bytes: u64,
    pub required_disk_reserve_bytes: u64,
    pub memory_full_psi_avg10_percent: f64,
    pub swap_in_pages_delta: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg(feature = "appliance")]
pub struct VerifiedQualification {
    pub plan: QualificationPlanDocument,
    pub submission: QualificationSubmission,
}

/// Exact durable-operation identity paired with a signed qualification request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg(feature = "appliance")]
pub struct QualificationRunInput {
    pub operation_id: String,
    pub submission: QualificationSubmission,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "appliance", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
#[cfg(feature = "appliance")]
pub struct QualificationExecution {
    pub stdout_encoding: String,
    pub stdout_base64: String,
    pub stdout_bytes: u64,
    pub stdout_sha256: String,
    pub stderr_encoding: String,
    pub stderr_base64: String,
    pub stderr_bytes: u64,
    pub stderr_sha256: String,
    pub exit_code: i64,
}

#[cfg(feature = "appliance")]
impl QualificationExecution {
    pub fn from_output(stdout: &[u8], stderr: &[u8], exit_code: i64) -> Result<Self, String> {
        if stdout
            .len()
            .checked_add(stderr.len())
            .is_none_or(|raw| raw > MAX_OUTPUT_BYTES as usize)
        {
            return Err("qualification raw output exceeds the executor limit".into());
        }
        let stdout_base64 = BASE64.encode(
            zstd::stream::encode_all(Cursor::new(stdout), 3)
                .map_err(|_| "qualification stdout compression failed")?,
        );
        let stderr_base64 = BASE64.encode(
            zstd::stream::encode_all(Cursor::new(stderr), 3)
                .map_err(|_| "qualification stderr compression failed")?,
        );
        if stdout_base64
            .len()
            .checked_add(stderr_base64.len())
            .is_none_or(|encoded| encoded > MAX_ENCODED_OUTPUT_BYTES)
        {
            return Err("qualification encoded output exceeds the executor frame budget".into());
        }
        Ok(Self {
            stdout_encoding: "zstd+base64".into(),
            stdout_base64,
            stdout_bytes: stdout.len() as u64,
            stdout_sha256: sha256_identity(stdout),
            stderr_encoding: "zstd+base64".into(),
            stderr_base64,
            stderr_bytes: stderr.len() as u64,
            stderr_sha256: sha256_identity(stderr),
            exit_code,
        })
    }

    pub fn decode_stdout(&self) -> Result<Vec<u8>, String> {
        decode_output(
            &self.stdout_encoding,
            &self.stdout_base64,
            self.stdout_bytes,
            &self.stdout_sha256,
        )
    }

    pub fn decode_stderr(&self) -> Result<Vec<u8>, String> {
        decode_output(
            &self.stderr_encoding,
            &self.stderr_base64,
            self.stderr_bytes,
            &self.stderr_sha256,
        )
    }

    pub fn validate_output_identity(&self) -> Result<(), String> {
        if self
            .stdout_bytes
            .checked_add(self.stderr_bytes)
            .is_none_or(|bytes| bytes > MAX_OUTPUT_BYTES)
            || self
                .stdout_base64
                .len()
                .checked_add(self.stderr_base64.len())
                .is_none_or(|bytes| bytes > MAX_ENCODED_OUTPUT_BYTES)
        {
            return Err("qualification combined output exceeds the executor limit".into());
        }
        self.decode_stdout()?;
        self.decode_stderr()?;
        Ok(())
    }
}

#[cfg(feature = "appliance")]
fn decode_output(
    encoding: &str,
    encoded: &str,
    raw_bytes: u64,
    raw_sha256: &str,
) -> Result<Vec<u8>, String> {
    if encoding != "zstd+base64"
        || encoded.len() > MAX_ENCODED_OUTPUT_BYTES
        || raw_bytes > MAX_OUTPUT_BYTES
    {
        return Err("qualification output encoding is invalid".into());
    }
    let compressed = BASE64
        .decode(encoded)
        .map_err(|_| "qualification output base64 is invalid")?;
    let decoder = zstd::stream::read::Decoder::new(Cursor::new(compressed))
        .map_err(|_| "qualification output compression is invalid")?;
    let mut decoded = Vec::with_capacity(usize::try_from(raw_bytes).unwrap_or_default());
    decoder
        .take(raw_bytes.saturating_add(1))
        .read_to_end(&mut decoded)
        .map_err(|_| "qualification output compression is invalid")?;
    if decoded.len() as u64 != raw_bytes || sha256_identity(&decoded) != raw_sha256 {
        return Err("qualification output identity verification failed".into());
    }
    Ok(decoded)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "appliance", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
#[cfg(feature = "appliance")]
pub struct QualificationResultDocument {
    pub schema: String,
    pub outcome: String,
    pub plan: QualificationPlanDocument,
    pub execution: QualificationExecution,
}

#[cfg(feature = "appliance")]
impl QualificationResultDocument {
    pub fn new(plan: QualificationPlanDocument, execution: QualificationExecution) -> Self {
        let outcome = if execution.exit_code == 0 {
            "passed"
        } else {
            "failed"
        };
        Self {
            schema: QUALIFICATION_RESULT_SCHEMA.into(),
            outcome: outcome.into(),
            plan,
            execution,
        }
    }
}

#[cfg(feature = "appliance")]
pub fn verify_submission(
    submission: &QualificationSubmission,
    public_key: &str,
    host: &QualificationTarget,
) -> Result<VerifiedQualification, String> {
    if submission.manifest_base64.len() > MAX_MANIFEST_BYTES.saturating_mul(2) {
        return Err("qualification manifest exceeds the encoded size limit".into());
    }
    if submission.signature.len() > MAX_SIGNATURE_BYTES {
        return Err("qualification signature exceeds the size limit".into());
    }
    let bytes = BASE64
        .decode(&submission.manifest_base64)
        .map_err(|_| "qualification manifest base64 is invalid".to_owned())?;
    if bytes.is_empty() || bytes.len() > MAX_MANIFEST_BYTES {
        return Err("qualification manifest must contain 1..65536 bytes".into());
    }
    verify_signature(public_key, &submission.signature, &bytes)?;
    let manifest: QualificationManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("qualification manifest is invalid: {error}"))?;
    let image_digest = validate_manifest(&manifest, host)?;
    Ok(VerifiedQualification {
        plan: QualificationPlanDocument {
            schema: QUALIFICATION_PLAN_SCHEMA.into(),
            admitted: true,
            job_id: manifest.job_id,
            operation: manifest.operation,
            manifest_sha256: sha256_identity(&bytes),
            image: manifest.image,
            image_digest,
            target: manifest.target,
            limits: manifest.limits,
            isolation: QualificationIsolation::default(),
            resources: None,
        },
        submission: submission.clone(),
    })
}

#[cfg(feature = "appliance")]
pub fn admit_resources(
    plan: &mut QualificationPlanDocument,
    snapshot: &super::resources::HostResourceSnapshot,
    policy: &super::resources::ResourcePolicy,
    now_unix_ms: u64,
) -> Result<(), String> {
    if snapshot.observed_at_unix_ms > now_unix_ms
        || now_unix_ms.saturating_sub(snapshot.observed_at_unix_ms) > policy.max_snapshot_age_ms()
        || !snapshot.is_complete()
    {
        return Err("qualification requires a fresh complete resource snapshot".into());
    }
    let mem_available = snapshot
        .mem_available_bytes
        .ok_or_else(|| "qualification memory telemetry is missing".to_owned())?;
    let live_after = mem_available
        .checked_sub(plan.limits.memory_bytes)
        .ok_or_else(|| "qualification memory admission was rejected".to_owned())?;
    let floor = policy
        .system_reserve_bytes
        .max(policy.emergency_available_floor_bytes);
    let psi = snapshot
        .memory_full_psi_avg10_percent
        .filter(|value| value.is_finite())
        .ok_or_else(|| "qualification pressure telemetry is missing".to_owned())?;
    let swap = snapshot
        .swap_in_pages_delta
        .ok_or_else(|| "qualification swap telemetry is missing".to_owned())?;
    let disk_available = snapshot
        .disk_available_bytes
        .ok_or_else(|| "qualification disk telemetry is missing".to_owned())?;
    let disk_after = disk_available
        .checked_sub(plan.limits.image_bytes)
        .ok_or_else(|| "qualification image disk admission was rejected".to_owned())?;
    if live_after < floor
        || disk_after < policy.disk_reserve_bytes
        || psi >= policy.memory_full_psi_avg10_percent
        || swap > 0
    {
        return Err("qualification resource pressure admission was rejected".into());
    }
    plan.resources = Some(QualificationResourceAdmission {
        observed_at_unix_ms: snapshot.observed_at_unix_ms,
        mem_available_bytes: mem_available,
        live_available_after_start_bytes: live_after,
        required_available_floor_bytes: floor,
        disk_available_bytes: disk_available,
        disk_available_after_pull_bytes: disk_after,
        required_disk_reserve_bytes: policy.disk_reserve_bytes,
        memory_full_psi_avg10_percent: psi,
        swap_in_pages_delta: swap,
    });
    Ok(())
}

#[cfg(feature = "appliance")]
fn validate_manifest(
    manifest: &QualificationManifest,
    host: &QualificationTarget,
) -> Result<String, String> {
    if manifest.schema != QUALIFICATION_MANIFEST_SCHEMA {
        return Err("qualification manifest schema is unsupported".into());
    }
    if !valid_job_id(&manifest.job_id) {
        return Err("qualification job ID must be a lowercase slug".into());
    }
    let image_digest = image_digest(&manifest.image)
        .ok_or_else(|| "qualification image must pin one sha256 digest".to_owned())?;
    if &manifest.target != host {
        return Err("qualification target differs from the executor host identity".into());
    }
    validate_limits(&manifest.limits)?;
    Ok(image_digest.into())
}

#[cfg(feature = "appliance")]
fn valid_job_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

#[cfg(feature = "appliance")]
fn image_digest(image: &str) -> Option<&str> {
    if image.is_empty() || image.len() > 256 || image.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }
    let (repository, digest) = image.split_once("@sha256:")?;
    if repository.is_empty()
        || repository.contains('@')
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    Some(digest)
}

#[cfg(feature = "appliance")]
fn validate_limits(limits: &QualificationLimits) -> Result<(), String> {
    let valid = (1..=MAX_MEMORY_BYTES).contains(&limits.memory_bytes)
        && (1..=MAX_IMAGE_BYTES).contains(&limits.image_bytes)
        && (1..=MAX_PIDS).contains(&limits.pids)
        && (1..=MAX_CPU_CORES).contains(&limits.cpu_cores)
        && (1..=MAX_TIMEOUT_SECONDS).contains(&limits.timeout_seconds)
        && (1..=MAX_OUTPUT_BYTES).contains(&limits.output_bytes);
    valid
        .then_some(())
        .ok_or_else(|| "qualification resource limits exceed executor hard ceilings".into())
}

#[cfg(feature = "appliance")]
fn verify_signature(public_key: &str, signature: &str, message: &[u8]) -> Result<(), String> {
    use minisign_verify::{PublicKey, Signature};
    let lines = public_key.lines().collect::<Vec<_>>();
    let key = match lines.as_slice() {
        [raw] => PublicKey::from_base64(raw),
        [comment, _] if valid_public_key_comment(comment) => PublicKey::decode(public_key),
        _ => return Err("qualification release public key has an invalid shape".into()),
    }
    .map_err(|_| "qualification release public key is invalid".to_owned())?;
    let signature = Signature::decode(signature)
        .map_err(|_| "qualification manifest signature is invalid".to_owned())?;
    key.verify(message, &signature, false)
        .map_err(|_| "qualification manifest signature verification failed".to_owned())
}

#[cfg(feature = "appliance")]
fn valid_public_key_comment(comment: &str) -> bool {
    const PREFIXES: [&str; 2] = [
        "untrusted comment: minisign public key ",
        "untrusted comment: minisign public key: ",
    ];
    PREFIXES.iter().any(|prefix| {
        comment.strip_prefix(prefix).is_some_and(|identifier| {
            identifier.len() == 16 && identifier.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    })
}

#[cfg(feature = "appliance")]
fn sha256_identity(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(all(test, feature = "appliance"))]
mod tests {
    use super::{
        QUALIFICATION_PLAN_SCHEMA, QualificationExecution, QualificationIsolation,
        QualificationLimits, QualificationOperation, QualificationPlanDocument,
        QualificationSubmission, QualificationTarget, admit_resources,
    };
    use crate::spark::resources::{HostResourceSnapshot, ResourcePolicy};
    use base64::Engine as _;
    use std::io::Cursor;

    #[test]
    fn qualification_submission_rejects_runtime_injection_fields() {
        let hostile = serde_json::json!({
            "manifest_base64": "e30=",
            "signature": "invalid",
            "dry_run": true,
            "executable": "/bin/sh"
        });

        assert!(serde_json::from_value::<QualificationSubmission>(hostile).is_err());
    }

    #[test]
    fn qsa_qualification_maps_to_one_fixed_runner_operation() {
        let operation = QualificationOperation::SparkFlashQsaV1;
        assert_eq!(
            operation.executable(),
            "/opt/sparky/bin/sparky-qualification"
        );
        assert_eq!(operation.arguments(), &["qsa-target", "--json"]);
    }

    #[test]
    fn lifecycle_qualification_is_a_closed_precompiled_operation() {
        let operation: QualificationOperation =
            serde_json::from_str("\"spark_flash_cuda_lifecycle_v1\"").unwrap();
        assert_eq!(
            operation.executable(),
            "/opt/sparky/bin/sparky-qualification"
        );
        assert_eq!(operation.arguments(), &["cuda-lifecycle-target", "--json"]);
        assert!(operation.jit_environment().is_empty());
    }

    #[test]
    fn catalog_qualification_is_a_closed_precompiled_operation() {
        let operation: QualificationOperation =
            serde_json::from_str("\"spark_flash_kernel_catalog_v1\"").unwrap();
        assert_eq!(
            operation.executable(),
            "/opt/sparky/bin/sparky-qualification"
        );
        assert_eq!(operation.arguments(), &["kernel-catalog-target", "--json"]);
        assert!(operation.jit_environment().is_empty());
    }

    #[test]
    fn memory_qualification_is_a_closed_precompiled_operation() {
        let operation: QualificationOperation =
            serde_json::from_str("\"spark_flash_memory_admission_v1\"").unwrap();
        assert_eq!(
            operation.executable(),
            "/opt/sparky/bin/sparky-qualification"
        );
        assert_eq!(
            operation.arguments(),
            &["memory-admission-target", "--json"]
        );
        assert!(operation.jit_environment().is_empty());
    }

    #[test]
    fn graph_qualification_is_a_closed_precompiled_operation() {
        let operation: QualificationOperation =
            serde_json::from_str("\"spark_flash_graph_catalog_v1\"").unwrap();
        assert_eq!(
            operation.executable(),
            "/opt/sparky/bin/sparky-qualification"
        );
        assert_eq!(operation.arguments(), &["graph-catalog-target", "--json"]);
        assert!(operation.jit_environment().is_empty());
    }

    #[test]
    fn moe_qualification_maps_signed_wire_value_to_one_fixed_runner() {
        let operation: QualificationOperation =
            serde_json::from_str("\"spark_flash_moe_v1\"").unwrap();
        assert_eq!(
            operation.executable(),
            "/opt/sparky/bin/sparky-qualification"
        );
        assert_eq!(operation.arguments(), &["moe-target", "--json"]);
    }

    #[test]
    fn qualification_session_is_a_closed_precompiled_operation() {
        let operation: QualificationOperation =
            serde_json::from_str("\"spark_flash_session_v1\"").unwrap();
        assert_eq!(
            operation.executable(),
            "/opt/sparky/bin/sparky-qualification"
        );
        assert_eq!(operation.arguments(), &["session-target", "--json"]);
        assert!(operation.jit_environment().is_empty());
    }

    #[test]
    fn qualification_model_prefix_is_a_closed_precompiled_operation() {
        let wire = "\"spark_flash_model_prefix_v1\"";
        let operation: QualificationOperation = serde_json::from_str(wire).unwrap();
        assert_eq!(serde_json::to_string(&operation).unwrap(), wire);
        assert_eq!(
            operation.executable(),
            "/opt/sparky/bin/sparky-qualification"
        );
        assert_eq!(operation.arguments(), &["model-prefix-target", "--json"]);
        assert!(operation.jit_environment().is_empty());
    }

    #[test]
    fn qualification_model_prefix_rejects_command_payloads_and_unknown_versions() {
        for wire in [
            "\"spark_flash_model_prefix_v2\"",
            "{\"spark_flash_model_prefix_v1\":{\"argv\":[\"/bin/sh\"]}}",
        ] {
            assert!(serde_json::from_str::<QualificationOperation>(wire).is_err());
        }
    }

    #[test]
    fn qualification_manifest_requires_release_signature_and_exact_host() {
        let target = test_target();
        let manifest = super::QualificationManifest {
            schema: super::QUALIFICATION_MANIFEST_SCHEMA.into(),
            job_id: "spark-flash-adaptive-decoding".into(),
            operation: QualificationOperation::SparkFlashAdaptiveDecodingV1,
            image: format!("registry.example/sparky@sha256:{}", "1".repeat(64)),
            target: target.clone(),
            limits: QualificationLimits {
                memory_bytes: 8 * 1024 * 1024 * 1024,
                image_bytes: 16 * 1024 * 1024 * 1024,
                pids: 64,
                cpu_cores: 4,
                timeout_seconds: 60,
                output_bytes: 1024,
            },
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let signature = minisign::sign(None, &keys.sk, Cursor::new(&bytes), None, None)
            .unwrap()
            .into_string();
        let mut submission = QualificationSubmission {
            manifest_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
            signature,
            dry_run: true,
        };

        assert!(super::verify_submission(&submission, &keys.pk.to_base64(), &target).is_ok());
        let mut wrong_target = target.clone();
        wrong_target.driver_version = "different".into();
        assert!(
            super::verify_submission(&submission, &keys.pk.to_base64(), &wrong_target).is_err()
        );
        submission.manifest_base64.push('A');
        assert!(super::verify_submission(&submission, &keys.pk.to_base64(), &target).is_err());
    }

    #[test]
    fn qualification_execution_hashes_lossless_output() {
        let execution = QualificationExecution::from_output(b"trace\0", b"warning\n", 0).unwrap();

        assert_eq!(execution.stdout_bytes, 6);
        assert_eq!(execution.stderr_bytes, 8);
        assert_eq!(execution.decode_stdout().unwrap(), b"trace\0");
        assert_eq!(execution.decode_stderr().unwrap(), b"warning\n");
        assert!(execution.stdout_sha256.starts_with("sha256:"));
        assert!(execution.stderr_sha256.starts_with("sha256:"));
    }

    #[test]
    fn qualification_nonzero_exit_is_attributable_and_never_passes() {
        let execution = QualificationExecution::from_output(b"trace\n", b"failed\n", 17).unwrap();
        let result = super::QualificationResultDocument::new(
            plan_with_memory(8 * 1024 * 1024 * 1024),
            execution,
        );

        assert_eq!(result.outcome, "failed");
        assert_eq!(result.execution.exit_code, 17);
        assert_eq!(result.execution.decode_stderr().unwrap(), b"failed\n");
    }

    #[test]
    fn qualification_execution_round_trips_large_repetitive_trace() {
        let trace = vec![0x5a; 4 * 1024 * 1024];

        let execution = QualificationExecution::from_output(&trace, b"", 0).unwrap();

        assert_eq!(execution.decode_stdout().unwrap(), trace);
    }

    #[test]
    fn qualification_execution_rejects_incompressible_frame_overflow() {
        let mut state = 0x6a09_e667_f3bc_c908_u64;
        let output = (0..7 * 1024 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect::<Vec<_>>();

        assert!(QualificationExecution::from_output(&output, b"", 0).is_err());
    }

    #[test]
    fn qualification_execution_round_trips_bounded_numeric_capture() {
        let mut state = 0x243f_6a88_85a3_08d3_u64;
        let output = (0..4 * 1024 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect::<Vec<_>>();
        let execution = QualificationExecution::from_output(&output, b"", 0).unwrap();
        assert_eq!(execution.decode_stdout().unwrap(), output);
    }

    #[test]
    fn qualification_execution_decode_bounds_forged_compressed_length() {
        let mut execution =
            QualificationExecution::from_output(&vec![0; 4 * 1024 * 1024], b"", 0).unwrap();
        execution.stdout_bytes = 1;

        assert!(execution.decode_stdout().is_err());
    }

    #[test]
    fn qualification_execution_rejects_forged_combined_raw_limit() {
        let mut execution =
            QualificationExecution::from_output(&vec![0; 9 * 1024 * 1024], b"", 0).unwrap();
        execution.stderr_base64 = execution.stdout_base64.clone();
        execution.stderr_bytes = execution.stdout_bytes;
        execution.stderr_sha256 = execution.stdout_sha256.clone();

        assert!(execution.validate_output_identity().is_err());
    }

    #[test]
    fn qualification_resource_admission_uses_the_greater_live_memory_floor() {
        const OBSERVED_AT: u64 = 10_000;
        const AVAILABLE: u64 = 19_855_122_432;
        const JOB_MEMORY: u64 = AVAILABLE - 8 * 1024 * 1024 * 1024;
        let policy = ResourcePolicy::capacity_first();
        let snapshot = complete_snapshot(OBSERVED_AT, AVAILABLE);
        let mut plan = plan_with_memory(JOB_MEMORY);

        admit_resources(&mut plan, &snapshot, &policy, OBSERVED_AT).unwrap();
        assert_eq!(
            plan.resources.unwrap().required_available_floor_bytes,
            8 * 1024 * 1024 * 1024
        );

        let mut rejected = plan_with_memory(JOB_MEMORY + 1);
        assert!(admit_resources(&mut rejected, &snapshot, &policy, OBSERVED_AT).is_err());
    }

    #[test]
    fn qualification_resource_admission_preserves_disk_reserve_before_pull() {
        const OBSERVED_AT: u64 = 10_000;
        let policy = ResourcePolicy::capacity_first();
        let mut snapshot = complete_snapshot(OBSERVED_AT, 32 * 1024 * 1024 * 1024);
        snapshot.disk_available_bytes = Some(policy.disk_reserve_bytes + 1024);
        let mut plan = plan_with_memory(8 * 1024 * 1024 * 1024);

        admit_resources(&mut plan, &snapshot, &policy, OBSERVED_AT).unwrap();
        snapshot.disk_available_bytes = Some(policy.disk_reserve_bytes + 1023);
        assert!(admit_resources(&mut plan, &snapshot, &policy, OBSERVED_AT).is_err());
    }

    fn complete_snapshot(observed_at_unix_ms: u64, available: u64) -> HostResourceSnapshot {
        HostResourceSnapshot {
            schema: "sparkplane.resources.snapshot/v1".into(),
            observed_at_unix_ms,
            mem_total_bytes: Some(128 * 1024 * 1024 * 1024),
            mem_available_bytes: Some(available),
            memory_full_psi_avg10_percent: Some(0.0),
            swap_in_pages_delta: Some(0),
            disk_available_bytes: Some(1024 * 1024 * 1024 * 1024),
        }
    }

    fn plan_with_memory(memory_bytes: u64) -> QualificationPlanDocument {
        QualificationPlanDocument {
            schema: QUALIFICATION_PLAN_SCHEMA.into(),
            admitted: true,
            job_id: "spark-flash-adaptive-decoding".into(),
            operation: QualificationOperation::SparkFlashAdaptiveDecodingV1,
            manifest_sha256: format!("sha256:{}", "0".repeat(64)),
            image: format!("registry.example/sparky@sha256:{}", "1".repeat(64)),
            image_digest: "1".repeat(64),
            target: test_target(),
            limits: QualificationLimits {
                memory_bytes,
                image_bytes: 1024,
                pids: 64,
                cpu_cores: 4,
                timeout_seconds: 60,
                output_bytes: 1024,
            },
            isolation: QualificationIsolation::default(),
            resources: None,
        }
    }

    fn test_target() -> QualificationTarget {
        QualificationTarget {
            architecture: "aarch64".into(),
            gpu_model: "NVIDIA GB10".into(),
            compute_capability: "12.1".into(),
            dgx_build: "build".into(),
            driver_version: "driver".into(),
            toolkit_version: "toolkit".into(),
            protected_fingerprint: "fingerprint".into(),
        }
    }
}
