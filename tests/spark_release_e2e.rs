use std::{fs, path::Path};

#[test]
fn release_build_installs_all_pinned_toolchain_components() {
    let workflow = fs::read_to_string(".github/workflows/spark-release.yml").unwrap();
    let build = workflow.split("  build:\n").nth(1).unwrap();
    let setup = build
        .split("      - uses: actions/setup-python@")
        .next()
        .unwrap();
    let toolchain: toml::Value =
        toml::from_str(&fs::read_to_string("rust-toolchain.toml").unwrap()).unwrap();
    for component in toolchain["toolchain"]["components"].as_array().unwrap() {
        assert!(
            setup.contains(component.as_str().unwrap()),
            "build setup omits {component}"
        );
    }
}

#[test]
fn spark_release_inventory_and_policy_are_repository_owned() {
    let workflow = fs::read_to_string(".github/workflows/spark-release.yml").unwrap()
        + &fs::read_to_string("scripts/sign-spark-release.sh").unwrap();
    let policy = fs::read_to_string("deny.toml").unwrap();
    for required in [
        "cargo deny --no-default-features --features appliance check",
        "make lint-spark",
        "cargo auditable zigbuild",
        "aarch64-unknown-linux-gnu",
        "resolved-features.txt",
        "duplicate-dependencies.txt",
        "native-unsafe-build-inventory.txt",
        "SHA256SUMS",
        "minisign -Sm",
    ] {
        assert!(
            workflow.contains(required),
            "missing release gate: {required}"
        );
    }
    for required in ["[advisories]", "[licenses]", "[bans]", "[sources]"] {
        assert!(
            policy.contains(required),
            "missing dependency policy: {required}"
        );
    }
    assert!(policy.contains("ignore = []"));
}

#[test]
fn spark_supervision_and_lsm_assets_keep_the_split_boundary() {
    let target = fs::read_to_string("configs/systemd/system/sparkplane.target").unwrap();
    let agent = fs::read_to_string("configs/systemd/system/sparkplane-agent.service").unwrap();
    let executor =
        fs::read_to_string("configs/systemd/system/sparkplane-executor.service").unwrap();
    let agent_lsm = fs::read_to_string("configs/apparmor.d/sparkplane-agent").unwrap();
    let executor_lsm = fs::read_to_string("configs/apparmor.d/sparkplane-executor").unwrap();
    assert!(target.contains("Requires=sparkplane-executor.service sparkplane-agent.service"));
    assert!(agent.contains("PartOf=sparkplane.target"));
    assert!(executor.contains("PrivateNetwork=yes"));
    assert!(agent_lsm.contains("deny /var/run/docker.sock rw,"));
    assert!(executor_lsm.contains("deny network inet,"));
    assert!(Path::new("specs/openapi/sparkplane-control-v1.json").is_file());
}

#[test]
fn optional_disruptive_maintenance_is_never_encoded_as_automatic_work() {
    let installer = fs::read_to_string("src/spark/install.rs").unwrap();
    assert!(installer.contains("docker_restart: \"not_run\".into()"));
    assert!(installer.contains("host_reboot: \"not_run\".into()"));
    for forbidden in ["restart docker", "systemctl reboot", "shutdown -r"] {
        assert!(!installer.to_ascii_lowercase().contains(forbidden));
    }
}
