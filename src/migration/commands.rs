//! Closed set of host operations. No executable or argument list comes from a plan.
use anyhow::{Result, ensure};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
};

pub const FENCE: &str = "add table inet sparkplane_migration\nadd chain inet sparkplane_migration input { type filter hook input priority -10; policy accept; }\nadd rule inet sparkplane_migration input iifname != \"lo\" tcp dport 9843 ct state new reject\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Fence,
    FenceStatus,
    ListTables,
    SealTraffic,
    Unfence,
    Connections,
    ReloadUnits,
    StopOld,
    StartOld,
    StopNew,
    StartNew,
    EnableNew,
    DisableOld,
    RenameUser,
    RenameGroup,
    RestoreUser,
    RestoreGroup,
    ConfineAgent,
    ConfineExecutor,
}

impl Action {
    pub const ALL: [Self; 19] = [
        Self::Fence,
        Self::FenceStatus,
        Self::ListTables,
        Self::SealTraffic,
        Self::Unfence,
        Self::Connections,
        Self::ReloadUnits,
        Self::StopOld,
        Self::StartOld,
        Self::StopNew,
        Self::StartNew,
        Self::EnableNew,
        Self::DisableOld,
        Self::RenameUser,
        Self::RenameGroup,
        Self::RestoreUser,
        Self::RestoreGroup,
        Self::ConfineAgent,
        Self::ConfineExecutor,
    ];

    pub fn command(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Fence => ("/usr/sbin/nft", &["-f", "-"]),
            Self::SealTraffic => ("/usr/sbin/nft", &["-f", "-"]),
            Self::ListTables => ("/usr/sbin/nft", &["list", "tables"]),
            Self::FenceStatus => (
                "/usr/sbin/nft",
                &["list", "table", "inet", "sparkplane_migration"],
            ),
            Self::Unfence => (
                "/usr/sbin/nft",
                &["delete", "table", "inet", "sparkplane_migration"],
            ),
            Self::Connections => (
                "/usr/bin/ss",
                &["-Hnt", "state", "established", "sport", "=", ":9843"],
            ),
            Self::ReloadUnits => ("/usr/bin/systemctl", &["daemon-reload"]),
            Self::StopOld => (
                "/usr/bin/systemctl",
                &[
                    "stop",
                    "sy-spark-agent.service",
                    "sy-spark-executor.service",
                    "sy-spark.target",
                ],
            ),
            Self::StartOld => (
                "/usr/bin/systemctl",
                &[
                    "start",
                    "--no-block",
                    "sy-spark-executor.service",
                    "sy-spark-agent.service",
                    "sy-spark.target",
                ],
            ),
            Self::StopNew => (
                "/usr/bin/systemctl",
                &[
                    "stop",
                    "sparkplane-agent.service",
                    "sparkplane-executor.service",
                    "sparkplane.target",
                ],
            ),
            Self::StartNew => (
                "/usr/bin/systemctl",
                &[
                    "start",
                    "--no-block",
                    "sparkplane-executor.service",
                    "sparkplane-agent.service",
                    "sparkplane.target",
                ],
            ),
            Self::EnableNew => ("/usr/bin/systemctl", &["enable", "sparkplane.target"]),
            Self::DisableOld => ("/usr/bin/systemctl", &["disable", "sy-spark.target"]),
            Self::RenameUser => ("/usr/sbin/usermod", &["--login", "sparkplane", "sy-spark"]),
            Self::RenameGroup => (
                "/usr/sbin/groupmod",
                &["--new-name", "sparkplane", "sy-spark"],
            ),
            Self::RestoreUser => ("/usr/sbin/usermod", &["--login", "sy-spark", "sparkplane"]),
            Self::RestoreGroup => (
                "/usr/sbin/groupmod",
                &["--new-name", "sy-spark", "sparkplane"],
            ),
            Self::ConfineAgent => (
                "/usr/sbin/apparmor_parser",
                &["-r", "/etc/apparmor.d/sparkplane-agent"],
            ),
            Self::ConfineExecutor => (
                "/usr/sbin/apparmor_parser",
                &["-r", "/etc/apparmor.d/sparkplane-executor"],
            ),
        }
    }

    pub fn run(self) -> Result<Vec<u8>> {
        let (program, args) = self.command();
        let mut child = Command::new("/usr/bin/timeout")
            .args(["--kill-after=5s", "90s", program])
            .args(args)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        if matches!(self, Self::Fence | Self::SealTraffic) {
            child
                .stdin
                .take()
                .expect("piped stdin")
                .write_all(if matches!(self, Self::Fence) { FENCE.as_bytes() } else {
                    b"flush chain inet sparkplane_migration input\nadd rule inet sparkplane_migration input iifname != \"lo\" tcp dport 9843 reject\n"
                })?;
        }
        drop(child.stdin.take());
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut output = Vec::new();
        stdout
            .by_ref()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut output)?;
        // Keep draining without retaining bytes; the fixed timeout bounds the child.
        std::io::copy(&mut stdout, &mut std::io::sink())?;
        let status = child.wait()?;
        ensure!(
            status.success(),
            "fixed migration action {self:?} failed ({})",
            status
        );
        ensure!(
            output.len() <= 1024 * 1024,
            "migration command output exceeds limit"
        );
        Ok(output)
    }
}
