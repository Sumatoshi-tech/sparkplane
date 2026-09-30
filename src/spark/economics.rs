//! Reproducible public-price comparisons and private aggregate report storage.
use super::wire::LaunchSessionDocument;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub const SCHEMA: &str = "sparkplane.session-economics/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Price {
    pub model: String,
    pub source: String,
    pub verified_at: String,
    pub effective_from: String,
    pub input_microusd_per_million: u64,
    pub output_microusd_per_million: u64,
    pub long_context_threshold: Option<u64>,
    pub long_input_microusd_per_million: Option<u64>,
    pub long_output_microusd_per_million: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceCatalog {
    pub version: String,
    pub currency: String,
    pub tariff: String,
    pub models: Vec<Price>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Compression {
    pub status: String,
    pub commands: u64,
    pub raw_bytes: u64,
    pub filtered_bytes: u64,
    pub metrics_errors: u64,
}
impl Compression {
    pub fn saved_tokens(&self) -> u64 {
        self.raw_bytes
            .div_ceil(4)
            .saturating_sub(self.filtered_bytes.div_ceil(4))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparison {
    pub model: String,
    pub inference_usd_nanos: Option<u64>,
    pub estimated_rtk_usd_nanos: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub session_id: String,
    pub host: String,
    pub model: String,
    pub integration: String,
    pub eco_mode: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub exit_code: Option<i32>,
    pub inference: Option<LaunchSessionDocument>,
    pub compression: Compression,
    pub catalog: PriceCatalog,
    pub comparisons: Vec<Comparison>,
}
impl Report {
    pub fn calculate(&mut self) {
        self.comparisons = self
            .catalog
            .models
            .iter()
            .map(|price| {
                let inference = self.inference.as_ref().and_then(|session| {
                    let usage = &session.usage;
                    if (session.finished_at.is_none() && usage.requests == 0)
                        || (usage.requests > 0 && usage.unknown_usage_requests == usage.requests)
                    {
                        return None;
                    }
                    let short_input = usage
                        .input_tokens
                        .checked_sub(usage.long_context_input_tokens)?;
                    let short_output = usage
                        .output_tokens
                        .checked_sub(usage.long_context_output_tokens)?;
                    let long_in = price
                        .long_input_microusd_per_million
                        .unwrap_or(price.input_microusd_per_million);
                    let long_out = price
                        .long_output_microusd_per_million
                        .unwrap_or(price.output_microusd_per_million);
                    let value = u128::from(short_input)
                        * u128::from(price.input_microusd_per_million)
                        + u128::from(short_output) * u128::from(price.output_microusd_per_million)
                        + u128::from(usage.long_context_input_tokens) * u128::from(long_in)
                        + u128::from(usage.long_context_output_tokens) * u128::from(long_out);
                    u64::try_from(value / 1000).ok()
                });
                let rtk = (self.compression.metrics_errors == 0
                    && self.compression.status == "active")
                    .then(|| {
                        u64::try_from(
                            u128::from(self.compression.saved_tokens())
                                * u128::from(price.input_microusd_per_million)
                                / 1000,
                        )
                        .ok()
                    })
                    .flatten();
                Comparison {
                    model: price.model.clone(),
                    inference_usd_nanos: inference,
                    estimated_rtk_usd_nanos: rtk,
                }
            })
            .collect();
    }
}

pub fn catalog() -> Result<PriceCatalog> {
    Ok(serde_json::from_str(include_str!(
        "../../data/economics-prices.json"
    ))?)
}

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir()
            && meta.uid() == rustix::process::geteuid().as_raw()
            && meta.mode() & 0o077 == 0,
        "economics directory has unsafe ownership or permissions"
    );
    Ok(())
}

pub(crate) fn write_record(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("record has no parent"))?;
    private_dir(parent)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    let temporary = parent.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub(crate) fn read_record<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.uid() == rustix::process::geteuid().as_raw()
            && meta.mode() & 0o077 == 0
            && meta.len() <= 1024 * 1024,
        "invalid economics record"
    );
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "economics record too large");
    Ok(serde_json::from_slice(&bytes)?)
}

fn directory(config: &Path, host: &str) -> PathBuf {
    config
        .join("economics")
        .join(format!("{:x}", Sha256::digest(host.as_bytes())))
}

pub fn save(config: &Path, report: &Report) -> Result<()> {
    ensure!(
        report.session_id.parse::<ulid::Ulid>().is_ok(),
        "invalid session id"
    );
    let dir = directory(config, &report.host);
    private_dir(&dir)?;
    write_record(&dir.join(format!("{}.json", report.session_id)), report)?;
    prune(&dir)
}

fn prune(dir: &Path) -> Result<()> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(90);
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s == "json")
            && let Ok(old) = read_record::<Report>(&path)
            && chrono::DateTime::parse_from_rfc3339(&old.started_at).is_ok_and(|t| t < cutoff)
        {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub fn load(config: &Path, host: &str, id: Option<&str>) -> Result<Report> {
    let dir = directory(config, host);
    // Enforce retention even when no further launch writes a report.
    ensure!(
        fs::symlink_metadata(&dir)?.is_dir(),
        "invalid economics directory"
    );
    private_dir(&dir)?;
    prune(&dir)?;
    let path = if let Some(id) = id {
        ensure!(id.parse::<ulid::Ulid>().is_ok(), "invalid session id");
        dir.join(format!("{id}.json"))
    } else {
        let mut paths = fs::read_dir(&dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|s| s == "json")
                    && p.file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| s.parse::<ulid::Ulid>().is_ok())
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths
            .pop()
            .ok_or_else(|| anyhow::anyhow!("no retained economics reports"))?
    };
    let report: Report = read_record(&path)?;
    ensure!(
        report.schema == SCHEMA && report.host == host,
        "economics report identity mismatch"
    );
    Ok(report)
}

fn money(value: Option<u64>) -> String {
    match value {
        None => "unavailable".into(),
        Some(0) => "$0.00".into(),
        Some(v) if v < 10_000_000 => "<$0.01".into(),
        Some(v) => {
            let cents = (u128::from(v) + 5_000_000) / 10_000_000;
            format!("${}.{:02}", cents / 100, cents % 100)
        }
    }
}

pub fn render(report: &Report, fancy: bool, width: usize) -> String {
    let width = width.clamp(40, 100);
    let title = if fancy {
        "\x1b[1;32mSession economics\x1b[0m"
    } else {
        "Session economics"
    };
    let mut text = format!("\n{}\n{title}\n", "─".repeat(width));
    text.push_str(&format!(
        "{} · {} · exit {}\n",
        report.integration,
        report.model,
        report
            .exit_code
            .map(|v| v.to_string())
            .unwrap_or_else(|| "interrupted".into())
    ));
    if let (Ok(start), Some(end)) = (
        chrono::DateTime::parse_from_rfc3339(&report.started_at),
        report
            .finished_at
            .as_ref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()),
    ) {
        let seconds = (end - start).num_seconds().max(0);
        text.push_str(&format!("Duration: {}m {}s\n", seconds / 60, seconds % 60));
    }
    if let Some(session) = &report.inference {
        let usage = &session.usage;
        let incomplete = usage.unknown_usage_requests > 0
            || usage.pending_requests > 0
            || usage.accounting_errors > 0
            || session.finished_at.is_none();
        text.push_str(&format!(
            "Tokens burned{}: {} input + {} output = {} total · {} requests\n",
            if incomplete { " (known)" } else { "" },
            usage.input_tokens,
            usage.output_tokens,
            usage.input_tokens.saturating_add(usage.output_tokens),
            usage.requests
        ));
        if usage.unknown_usage_requests > 0
            || usage.pending_requests > 0
            || usage.accounting_errors > 0
            || session.finished_at.is_none()
        {
            text.push_str("Inference accounting incomplete; known usage is a lower bound.\n");
        }
    } else {
        text.push_str("Inference usage unavailable.\n");
    }
    let rtk = &report.compression;
    text.push_str(&format!(
        "RTK: {} · {} commands\n",
        rtk.status, rtk.commands
    ));
    if rtk.metrics_errors > 0 {
        text.push_str("RTK metrics incomplete; dollar savings unavailable.\n");
    }
    if rtk.commands > 0 {
        let pct = if rtk.raw_bytes > 0 {
            100.0 * (1.0 - rtk.filtered_bytes as f64 / rtk.raw_bytes as f64)
        } else {
            0.0
        };
        text.push_str(&format!(
            "Tool output: ~{} → ~{} tokens · {:.1}% smaller · ~{} tokens saved\n",
            rtk.raw_bytes.div_ceil(4),
            rtk.filtered_bytes.div_ceil(4),
            pct,
            rtk.saved_tokens()
        ));
        if fancy {
            let kept = rtk
                .filtered_bytes
                .saturating_mul(20)
                .checked_div(rtk.raw_bytes)
                .unwrap_or(0)
                .min(20) as usize;
            text.push_str(&format!(
                "[{}{}]\n",
                "█".repeat(kept),
                "░".repeat(20 - kept)
            ));
        }
    }
    text.push_str("\nCloud equivalent / estimated RTK savings (USD):\n");
    for row in &report.comparisons {
        text.push_str(&format!(
            "  {:<20} {:>11} / {:>11}\n",
            row.model,
            money(row.inference_usd_nanos),
            money(row.estimated_rtk_usd_nanos)
        ));
    }
    text.push_str(&format!(
        "Public standard uncached rates · verified {}\n",
        report
            .catalog
            .models
            .first()
            .map(|p| p.verified_at.as_str())
            .unwrap_or("unknown")
    ));
    text.push_str("Estimates use local token counts; RTK is valued once at input rates.\nCloud comparisons exclude electricity/hardware and are not added together.\n");
    text
}

pub fn display(report: &Report) {
    use std::io::IsTerminal;
    eprint!(
        "{}",
        render(
            report,
            io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
            std::env::var("COLUMNS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(80)
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn money_rounds_and_tiny_values_are_visible() {
        assert_eq!(money(Some(1)), "<$0.01");
        assert_eq!(money(None), "unavailable");
        assert_eq!(money(Some(999_000_000)), "$1.00");
    }
    #[test]
    fn catalog_has_official_sources_and_known_context_tier() {
        let prices = catalog().unwrap();
        assert_eq!(prices.models.len(), 3);
        assert_eq!(prices.models[0].long_context_threshold, Some(272_000));
    }

    fn report() -> Report {
        let now = chrono::Utc::now().to_rfc3339();
        Report {
            schema: SCHEMA.into(),
            session_id: ulid::Ulid::new().to_string(),
            host: "fixture".into(),
            model: "local-model".into(),
            integration: "codex".into(),
            eco_mode: "max".into(),
            started_at: now.clone(),
            finished_at: Some(now.clone()),
            exit_code: Some(0),
            inference: Some(LaunchSessionDocument {
                schema: "sparkplane.launch-session/v1".into(),
                id: ulid::Ulid::new().to_string(),
                instance: "fixture".into(),
                model: "local-model".into(),
                integration: "codex".into(),
                eco_mode: "max".into(),
                started_at: now.clone(),
                finished_at: Some(now),
                usage: super::super::wire::SessionUsage {
                    requests: 1,
                    input_tokens: 272001,
                    output_tokens: 4,
                    long_context_input_tokens: 272001,
                    long_context_output_tokens: 4,
                    ..Default::default()
                },
            }),
            compression: Compression {
                status: "active".into(),
                commands: 1,
                raw_bytes: 400,
                filtered_bytes: 100,
                metrics_errors: 0,
            },
            catalog: catalog().unwrap(),
            comparisons: Vec::new(),
        }
    }

    #[test]
    fn costs_use_each_requests_context_tier_and_keep_rtk_separate() {
        let mut report = report();
        report.calculate();
        assert_eq!(
            report.comparisons[0].inference_usd_nanos,
            Some(1_088_064_000)
        );
        assert_eq!(report.comparisons[0].estimated_rtk_usd_nanos, Some(150_000));
        assert_eq!(report.comparisons[1].inference_usd_nanos, Some(816_063_000));
        let usage = &mut report.inference.as_mut().unwrap().usage;
        usage.input_tokens = 272000;
        usage.long_context_input_tokens = 0;
        usage.long_context_output_tokens = 0;
        report.calculate();
        assert_eq!(report.comparisons[0].inference_usd_nanos, Some(544_040_000));
        report
            .inference
            .as_mut()
            .unwrap()
            .usage
            .unknown_usage_requests = 1;
        report.compression.metrics_errors = 1;
        report.calculate();
        assert_eq!(report.comparisons[0].estimated_rtk_usd_nanos, None);
        let text = render(&report, false, 40);
        assert!(text.contains("lower bound"));
        assert!(!text.contains('\x1b'));
    }

    #[test]
    fn reports_are_private_scoped_and_expire_on_read() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let mut report = report();
        report.calculate();
        save(root.path(), &report).unwrap();
        assert_eq!(
            load(root.path(), "fixture", None).unwrap().session_id,
            report.session_id
        );
        assert!(load(root.path(), "other", Some(&report.session_id)).is_err());
        assert!(load(root.path(), "fixture", Some("../escape")).is_err());
        let path = directory(root.path(), "fixture").join(format!("{}.json", report.session_id));
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load(root.path(), "fixture", None).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let link = path.with_extension("link");
        symlink(&path, &link).unwrap();
        assert!(read_record::<Report>(&link).is_err());
        report.started_at = (chrono::Utc::now() - chrono::Duration::days(91)).to_rfc3339();
        write_record(&path, &report).unwrap();
        assert!(load(root.path(), "fixture", None).is_err());
        assert!(!path.exists());
    }
}
