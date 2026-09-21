use std::process::Command;

#[test]
fn launch_help_exposes_opt_in_network_access() {
    let output = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
        .args(["dgx-spark", "launch", "codex", "--help"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("--allow-network"));
}

#[test]
fn network_opt_in_rejects_shadowing_claude_settings_before_contacting_spark() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
        .env_clear()
        .env("SPARKPLANE_CONFIG_DIR", root.path())
        .args([
            "dgx-spark",
            "launch",
            "claude",
            "--allow-network",
            "--",
            "--settings",
            "{}",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--allow-network manages Claude --settings")
    );
}

#[test]
fn network_grants_cannot_be_saved_or_combined_with_restore() {
    for mode in ["--config", "--restore"] {
        for flag in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let mut command = Command::new(env!("CARGO_BIN_EXE_sparkplane"));
            command
                .env_clear()
                .env("SPARKPLANE_CONFIG_DIR", root.path())
                .args(["dgx-spark", "launch", "codex", mode]);
            if flag {
                command.arg("--allow-network");
            } else {
                command.env("SPARKPLANE_LAUNCH_ALLOW_NETWORK", "true");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(2));
            assert!(String::from_utf8_lossy(&output.stderr).contains("--allow-network"));
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }
}

#[test]
fn bridge_protocol_is_available_without_a_host_or_configuration() {
    let output = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
        .arg("--bridge-protocol")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"sparkplane.bridge/v1\n");
}

#[test]
fn standalone_help_has_no_sy_command_prefix() {
    let output = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
        .args(["dgx-spark", "install", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("sparkplane dgx-spark install"));
    assert!(!help.contains("sy spark"));
}

#[test]
fn appliance_migration_is_feature_gated_and_requires_one_explicit_mode() {
    let invoke = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_sparkplane"))
            .args(args)
            .env_clear()
            .output()
            .unwrap()
    };
    let help = invoke(&["bootstrap", "migrate-appliance", "--help"]);
    assert_eq!(help.status.success(), cfg!(feature = "appliance"));
    if cfg!(feature = "appliance") {
        assert!(String::from_utf8_lossy(&help.stdout).contains("--recovery-approval"));
    }
    for args in [
        vec!["bootstrap", "migrate-appliance"],
        vec!["bootstrap", "migrate-appliance", "--yes", "--dry-run"],
        vec!["dgx-spark", "migrate-appliance", "--yes"],
        vec![
            "bootstrap",
            "migrate-appliance",
            "--yes",
            "--recovery-approval",
            "approval.json",
        ],
        vec![
            "bootstrap",
            "migrate-appliance",
            "--recover",
            "--recovery-approval",
            "approval.json",
        ],
    ] {
        assert!(!invoke(&args).status.success());
    }
}
