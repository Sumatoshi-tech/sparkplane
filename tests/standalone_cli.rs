use std::process::Command;

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
