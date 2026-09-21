//! Local root bootstrap entrypoint; no arbitrary remote runner or destination.
use super::{
    appliance,
    container::{Namespace, Network},
    host::{Host, Linux, Platform, Record},
    journal::Journal,
    publication::write_private,
    release::Release,
    runner,
};
use anyhow::{Context, Result, ensure};
use clap::Args;
use std::{
    fs,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("mode").required(true).multiple(false).args(["dry_run", "yes", "recover", "resume"])))]
pub struct MigrationArgs {
    /// Verified ARM64 appliance bundle; required for planning and applying.
    #[arg(long, env = "SPARKPLANE_MIGRATION_BUNDLE")]
    pub bundle: Option<PathBuf>,
    /// Installed-authority-signed, host-bound trust transition document.
    #[arg(long, env = "SPARKPLANE_TRUST_TRANSITION")]
    pub transition: Option<PathBuf>,
    /// Detached Minisign signature of the exact transition document.
    #[arg(long, env = "SPARKPLANE_TRUST_SIGNATURE")]
    pub transition_signature: Option<PathBuf>,
    /// Validate signatures and local preconditions without stopping services.
    #[arg(long, env = "SPARKPLANE_DRY_RUN")]
    pub dry_run: bool,
    /// Drain clients and apply the reviewed, journaled transition.
    #[arg(long, env = "SPARKPLANE_YES")]
    pub yes: bool,
    /// Restore an interrupted pre-commit transaction; never restores after commit.
    #[arg(long, env = "SPARKPLANE_MIGRATION_RECOVER")]
    pub recover: bool,
    /// Installed-authority-signed approval of an exact post-restoration recovery handoff.
    #[arg(long, env = "SPARKPLANE_RECOVERY_APPROVAL", requires_all = ["recover", "recovery_signature"])]
    pub recovery_approval: Option<PathBuf>,
    /// Detached signature of --recovery-approval; never authorizes a forward migration.
    #[arg(
        long,
        env = "SPARKPLANE_RECOVERY_SIGNATURE",
        requires = "recovery_approval"
    )]
    pub recovery_signature: Option<PathBuf>,
    /// Finish opening traffic after a committed transaction; never reapplies state.
    #[arg(long, env = "SPARKPLANE_MIGRATION_RESUME")]
    pub resume: bool,
    /// Emit a machine-readable plan or result; diagnostics remain on stderr.
    #[arg(long, env = "SPARKPLANE_JSON")]
    pub json: bool,
}

fn private_directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && meta.uid() == 0 && meta.mode() & 0o077 == 0,
        "migration state directory must be root-owned and private"
    );
    Ok(())
}

fn old_authority() -> Result<String> {
    let release = Path::new("/opt/sy-spark/current").canonicalize()?;
    ensure!(
        release.parent() == Some(Path::new("/opt/sy-spark/releases")),
        "installed release link escapes its authority directory"
    );
    for path in [
        Path::new("/opt"),
        Path::new("/opt/sy-spark"),
        Path::new("/opt/sy-spark/releases"),
        &release,
    ] {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_dir() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0,
            "installed authority directory is not protected"
        );
    }
    let path = release.join("minisign.pub");
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0,
        "installed authority is not protected"
    );
    let key = String::from_utf8(super::release::read(&path, 4096)?)?;
    let key = key
        .lines()
        .find(|line| !line.starts_with("untrusted comment:") && !line.trim().is_empty())
        .context("installed public key missing")?
        .trim();
    minisign_verify::PublicKey::from_base64(key)?;
    Ok(key.into())
}

fn self_digest() -> Result<String> {
    Ok(appliance::digest(&super::release::read(
        &std::env::current_exe()?,
        512 * 1024 * 1024,
    )?))
}

fn recovery_approval(
    args: &MigrationArgs,
    record: &[u8],
    host: &str,
    executable: &str,
) -> Result<Option<(super::recovery::Approval, Vec<u8>, String)>> {
    let Some(path) = &args.recovery_approval else {
        return Ok(None);
    };
    ensure!(args.recover && !args.resume, "handoff is recovery-only");
    let bytes = super::release::read(path, 65536)?;
    let signature = String::from_utf8(super::release::read(
        args.recovery_signature
            .as_ref()
            .context("recovery signature required")?,
        4096,
    )?)?;
    let approval = super::recovery::Approval::verify(
        &bytes,
        &signature,
        &old_authority()?,
        [host, &appliance::digest(record), executable],
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
    )?;
    Ok(Some((approval, bytes, signature)))
}

fn receipt(work: &Path, bytes: &[u8], signature: &[u8]) -> Result<()> {
    for (suffix, value) in [("json", bytes), ("minisig", signature)] {
        let path = work.join(format!(
            "recovery-approval-{}.{suffix}",
            appliance::digest(bytes)
        ));
        if path.try_exists()? {
            ensure!(
                super::release::read(&path, 65536)? == value,
                "recovery receipt changed"
            );
        } else {
            write_private(&path, value)?;
        }
    }
    Ok(())
}

pub fn dispatch(host: &str, args: MigrationArgs) -> Result<()> {
    ensure!(
        host == "bootstrap",
        "appliance migration is a local bootstrap operation; invoke it on the Spark host"
    );
    ensure!(
        rustix::process::geteuid().as_raw() == 0,
        "appliance migration requires local root privileges"
    );
    let root = Path::new("/");
    let work = appliance::active_directory();
    let base = Path::new(appliance::BASE);
    let mut platform = Linux::connect()?;
    if args.recover || args.resume {
        ensure!(
            args.bundle.is_none()
                && args.transition.is_none()
                && args.transition_signature.is_none(),
            "recovery uses only the protected stored transaction"
        );
        private_directory(base)?;
        let _lock = lock(base)?;
        private_directory(&work)?;
        let record_bytes = super::release::read(&work.join("record.json"), 8 * 1024 * 1024)?;
        let mut record: Record = serde_json::from_slice(&record_bytes)?;
        let executable = self_digest()?;
        ensure!(
            record.plan.schema == "sparkplane.appliance-migration/v1"
                && record.plan.host == appliance::host_identity(root)?,
            "recovery requires the original approved host and schema"
        );
        let approval = recovery_approval(&args, &record_bytes, &record.plan.host, &executable)?;
        ensure!(
            approval.is_some() || record.plan.executable == executable,
            "recovery requires the original executable or signed handoff"
        );
        let mut journal = Journal::open(&work, &record.plan.host, &record.plan.release)?;
        if let Some((approval, bytes, signature)) = approval {
            ensure!(
                journal.restored_checkpoint(),
                "recovery handoff requires restored pre-commit state"
            );
            approval.apply(&mut record.plan, &platform.inventory(Namespace::Legacy)?)?;
            receipt(&work, &bytes, signature.as_bytes())?;
        }
        let mut appliance = Host {
            root: root.into(),
            work: work.clone(),
            record,
            platform,
        };
        if args.recover {
            ensure!(
                !journal.committed()?,
                "migration committed; use --resume, never restore pre-commit state"
            );
            appliance.restore_fence()?;
            runner::recover(&mut journal, &mut appliance)?;
            fs::rename(
                &work,
                base.join(format!("recovered-{}", uuid::Uuid::new_v4())),
            )?;
            fs::File::open(base)?.sync_all()?;
            report(args.json, "recovered", None)?;
        } else {
            ensure!(
                journal.committed()?,
                "interrupted pre-commit migration must use --recover"
            );
            if !work.join("opened.json").try_exists()? {
                appliance.restore_fence()?;
                appliance
                    .platform
                    .action(super::commands::Action::ReloadUnits)?;
                appliance
                    .platform
                    .action(super::commands::Action::StartNew)?;
                runner::run(&mut journal, &mut appliance)?;
                write_private(&work.join("opened.json"), b"{\"committed\":true}\n")?;
            } else {
                appliance
                    .platform
                    .healthy(root, &appliance.record.plan, false)?;
            }
            report(args.json, "committed", None)?;
        }
        return Ok(());
    }
    ensure!(
        !work.try_exists()? && !work.is_symlink(),
        "existing migration transaction requires --recover or --resume"
    );
    let bundle = args.bundle.context("--bundle is required")?;
    let transition =
        super::release::read(&args.transition.context("--transition is required")?, 16384)?;
    let signature = String::from_utf8(super::release::read(
        &args
            .transition_signature
            .context("--transition-signature is required")?,
        4096,
    )?)?;
    let manifest = super::release::read(&bundle.join("SHA256SUMS"), 65536)?;
    let authority = super::verify_transition(
        &transition,
        &signature,
        &old_authority()?,
        &appliance::host_identity(root)?,
        &appliance::digest(&manifest),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
    )?;
    let release = Release::load(&bundle, &authority.new_public_key)?;
    ensure!(
        appliance::digest(&release.manifest) == authority.release_sha256
            && release.executable_sha256 == self_digest()?,
        "release changed after approval or bootstrap executable differs"
    );
    ensure!(
        platform.inventory(Namespace::Current)?.is_empty(),
        "destination namespace already owns containers"
    );
    let mut plan = appliance::preflight(
        root,
        &release,
        &platform.inventory(Namespace::Legacy)?,
        appliance::available_disk(root)?,
    )?;
    ensure!(
        platform.network(Namespace::Current)?.is_none(),
        "destination network already exists"
    );
    if let Some(value) = platform.network(Namespace::Legacy)? {
        let network = Network::capture(&value, Namespace::Legacy)?;
        let attached = value["Containers"]
            .as_object()
            .context("network attachment inventory missing")?;
        ensure!(
            attached
                .keys()
                .all(|id| plan.active.iter().any(|a| a.container.container_id == *id)),
            "legacy network has an unaccounted attachment"
        );
        plan.legacy_network = Some(network);
    }
    ensure!(
        plan.active.is_empty() || plan.legacy_network.is_some(),
        "running instances have no owned network"
    );
    let tables = platform.action(super::commands::Action::ListTables)?;
    ensure!(
        !String::from_utf8(tables)?
            .lines()
            .any(|line| line.trim() == "table inet sparkplane_migration"),
        "migration firewall table already exists"
    );
    if args.dry_run {
        return report(args.json, "planned", Some(&plan));
    }
    ensure!(args.yes, "migration confirmation is required");
    if !base.try_exists()? {
        fs::create_dir(base)?;
        fs::set_permissions(base, fs::Permissions::from_mode(0o700))?;
        fs::File::open(base.parent().expect("fixed migration parent"))?.sync_all()?;
    }
    private_directory(base)?;
    let _lock = lock(base)?;
    ensure!(
        !work.try_exists()? && !work.is_symlink(),
        "another migration was staged concurrently"
    );
    let staging = tempfile::Builder::new()
        .prefix(".prepare-")
        .tempdir_in(base)?;
    let publication = appliance::stage(root, staging.path(), &plan, &release)?;
    let record = Record { plan, publication };
    write_private(
        &staging.path().join("record.json"),
        &serde_json::to_vec(&record)?,
    )?;
    write_private(&staging.path().join("trust-transition.json"), &transition)?;
    write_private(
        &staging.path().join("trust-transition.minisig"),
        signature.as_bytes(),
    )?;
    for entry in walkdir::WalkDir::new(staging.path())
        .contents_first(true)
        .follow_links(false)
    {
        let entry = entry?;
        if !entry.file_type().is_symlink() {
            fs::File::open(entry.path())?.sync_all()?;
        }
    }
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        staging.path(),
        rustix::fs::CWD,
        &work,
        rustix::fs::RenameFlags::NOREPLACE,
    )?;
    fs::File::open(base)?.sync_all()?;
    let mut journal = Journal::open(&work, &record.plan.host, &record.plan.release)?;
    let mut appliance = Host {
        root: root.into(),
        work: work.clone(),
        record,
        platform,
    };
    runner::run(&mut journal, &mut appliance)?;
    write_private(&work.join("opened.json"), b"{\"committed\":true}\n")?;
    report(args.json, "committed", None)
}

fn lock(base: &Path) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(base.join("migration.lock"))?;
    ensure!(
        file.metadata()?.is_file()
            && file.metadata()?.uid() == 0
            && file.metadata()?.mode() & 0o077 == 0,
        "migration lock is not protected"
    );
    file.try_lock()?;
    Ok(file)
}

fn report(json: bool, state: &str, plan: Option<&appliance::Plan>) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string(
                &serde_json::json!({"schema":"sparkplane.migration-result/v1","state":state,"plan":plan})
            )?
        );
    } else {
        println!("Appliance migration: {state}");
    }
    Ok(())
}
