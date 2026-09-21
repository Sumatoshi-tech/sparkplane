#![cfg(feature = "appliance")]
use sparkplane::migration::fence;

#[test]
fn model_warmup_fits_inside_agent_startup_deadline() {
    let unit = include_str!("../configs/systemd/system/sparkplane-agent.service");
    assert!(unit.lines().any(|line| line == "TimeoutStartSec=1900s"));
}

#[test]
fn recovery_extends_startup_without_rewriting_original_guards() {
    let root = tempfile::tempdir().unwrap();
    fence::install_guards(root.path()).unwrap();
    let path = root
        .path()
        .join("etc/systemd/system/sy-spark-agent.service.d/91-sparkplane-migration-startup.conf");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[Service]\nTimeoutStartSec=1900s\n"
    );
    fence::remove_guards(root.path()).unwrap();
    assert!(!path.exists());
}

#[test]
fn reboot_guard_requires_a_volatile_permit_for_both_namespaces() {
    let root = tempfile::tempdir().unwrap();
    fence::install_guards(root.path()).unwrap();
    for name in [
        "sy-spark-agent",
        "sy-spark-executor",
        "sparkplane-agent",
        "sparkplane-executor",
    ] {
        let path = root.path().join(format!(
            "etc/systemd/system/{name}.service.d/90-sparkplane-migration.conf"
        ));
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "[Unit]\nConditionPathExists=/run/sparkplane-migration-permit\n"
        );
    }
    assert!(!root.path().join("run/sparkplane-migration-permit").exists());
    fence::install_guards(root.path()).unwrap();
    fence::remove_guards(root.path()).unwrap();
    fence::remove_guards(root.path()).unwrap();
}

#[test]
fn guard_conflict_is_preserved() {
    let root = tempfile::tempdir().unwrap();
    let dir = root
        .path()
        .join("etc/systemd/system/sy-spark-agent.service.d");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("90-sparkplane-migration.conf");
    std::fs::write(&path, "operator-owned").unwrap();
    assert!(fence::install_guards(root.path()).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "operator-owned");
}

#[test]
fn later_drop_in_symlink_is_rejected_before_any_guard_is_created() {
    let root = tempfile::tempdir().unwrap();
    let systemd = root.path().join("etc/systemd/system");
    std::fs::create_dir_all(&systemd).unwrap();
    std::os::unix::fs::symlink(root.path(), systemd.join("sparkplane-executor.service.d")).unwrap();
    assert!(fence::install_guards(root.path()).is_err());
    assert!(!systemd.join("sy-spark-agent.service.d").exists());
}
