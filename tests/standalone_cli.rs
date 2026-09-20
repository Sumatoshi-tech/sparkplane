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
