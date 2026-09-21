use sha2::{Digest, Sha256};
use std::{fs, process::Command};

#[test]
fn protected_release_signs_with_an_encrypted_key_without_a_terminal() {
    assert!(
        fs::read_to_string(".github/workflows/spark-release.yml")
            .unwrap()
            .contains("bash scripts/sign-spark-release.sh release")
    );
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("appliance")).unwrap();
    fs::write(
        root.path().join("appliance/sparkplane-aarch64"),
        b"appliance",
    )
    .unwrap();
    fs::write(
        root.path().join("appliance/SHA256SUMS"),
        format!("{:x}  sparkplane-aarch64\n", Sha256::digest(b"appliance")),
    )
    .unwrap();
    for name in [
        "sparkplane-x86_64-unknown-linux-gnu",
        "sparkplane-aarch64-unknown-linux-gnu",
    ] {
        fs::write(root.path().join(name), b"fixture executable").unwrap();
    }
    let keys =
        minisign::KeyPair::generate_encrypted_keypair(Some("test-only signing password".into()))
            .unwrap();
    let output = Command::new("bash")
        .args([
            "scripts/sign-spark-release.sh",
            root.path().to_str().unwrap(),
        ])
        .env(
            "SPARKPLANE_MINISIGN_SECRET_KEY",
            keys.sk.to_box(None).unwrap().to_string(),
        )
        .env("MINISIGN_PASSWORD", "test-only signing password")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("test-only signing password"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("test-only signing password"));
    let public = minisign_verify::PublicKey::from_base64(&keys.pk.to_base64()).unwrap();
    for name in ["SHA256SUMS", "appliance/SHA256SUMS"] {
        let signature = fs::read_to_string(root.path().join(format!("{name}.minisig"))).unwrap();
        public
            .verify(
                &fs::read(root.path().join(name)).unwrap(),
                &minisign_verify::Signature::decode(&signature).unwrap(),
                false,
            )
            .unwrap();
    }
}
