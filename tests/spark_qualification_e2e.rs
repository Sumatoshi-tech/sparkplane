#![cfg(feature = "appliance")]

use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    process::Command,
    thread,
};

#[test]
fn public_cli_dry_run_posts_only_signed_manifest_material() {
    for operation in [
        "spark_flash_adaptive_decoding_v1",
        "spark_flash_qsa_v1",
        "spark_flash_moe_v1",
        "spark_flash_kernel_catalog_v1",
        "spark_flash_cuda_lifecycle_v1",
        "spark_flash_memory_admission_v1",
        "spark_flash_graph_catalog_v1",
        "spark_flash_session_v1",
        "spark_flash_model_prefix_v1",
    ] {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        fs::create_dir_all(config.join("spark")).unwrap();
        fs::create_dir_all(config.join("credentials/spark")).unwrap();
        let rcgen::CertifiedKey { cert, .. } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let ca = cert.pem();
        fs::write(config.join("spark/dgx.ca.pem"), &ca).unwrap();
        let pin = format!("sha256:{:x}", Sha256::digest(ca.as_bytes()));
        let credential = config.join("credentials/spark/dgx");
        fs::write(&credential, "fixture-token").unwrap();
        fs::set_permissions(&credential, fs::Permissions::from_mode(0o600)).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        fs::write(
        config.join("spark.toml"),
        format!(
            "[hosts.dgx]\nurl='http://{address}/'\nca_cert_sha256='{pin}'\ncredential='spark/dgx'\nrequest_timeout_seconds=5\n"
        ),
    )
    .unwrap();
        let manifest = root.path().join("qualification.json");
        let signature = root.path().join("qualification.minisig");
        fs::write(&manifest, br#"{"schema":"signed-fixture"}"#).unwrap();
        fs::write(&signature, "untrusted comment: fixture\nAAAA\n").unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
                if read == 0
                    || request.windows(4).any(|window| window == b"\r\n\r\n")
                        && request
                            .split(|byte| *byte == b'\n')
                            .filter_map(|line| std::str::from_utf8(line).ok())
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .is_some_and(|length| {
                                request
                                    .windows(4)
                                    .position(|window| window == b"\r\n\r\n")
                                    .is_some_and(|header| request.len() >= header + 4 + length)
                            })
                {
                    break;
                }
            }
            let text = String::from_utf8(request).unwrap();
            assert!(text.starts_with("POST /api/sparkplane/v1/qualifications HTTP/1.1"));
            assert!(
                text.to_ascii_lowercase()
                    .contains("authorization: bearer fixture-token")
            );
            let body = text.split("\r\n\r\n").nth(1).unwrap();
            let submitted: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(submitted["dry_run"], true);
            assert!(submitted.get("manifest_base64").is_some());
            assert!(submitted.get("signature").is_some());
            assert!(submitted.get("executable").is_none());
            let response = serde_json::json!({
            "schema":"sparkplane.qualification-plan/v1",
            "admitted":true,
            "job_id":"spark-flash-adaptive-decoding",
            "operation":operation,
            "manifest_sha256":format!("sha256:{}", "0".repeat(64)),
            "image":format!("registry.example/sparky@sha256:{}", "1".repeat(64)),
            "image_digest":"1".repeat(64),
            "target":{"architecture":"aarch64","gpu_model":"NVIDIA GB10","compute_capability":"12.1","dgx_build":"build","driver_version":"driver","toolkit_version":"toolkit","protected_fingerprint":"fingerprint"},
            "limits":{"memory_bytes":8589934592_u64,"image_bytes":17179869184_u64,"pids":64,"cpu_cores":4,"timeout_seconds":60,"output_bytes":4194304},
            "isolation":{"gpu_device":"0","network":"none","read_only_root":true,"privileged":false,"capabilities":[],"no_new_privileges":true,"restart":"no"},
            "resources":{"observed_at_unix_ms":1,"mem_available_bytes":19855122432_u64,"live_available_after_start_bytes":11265187840_u64,"required_available_floor_bytes":8589934592_u64,"disk_available_bytes":214748364800_u64,"disk_available_after_pull_bytes":197568495616_u64,"required_disk_reserve_bytes":107374182400_u64,"memory_full_psi_avg10_percent":0.0,"swap_in_pages_delta":0}
        })
        .to_string();
            write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .unwrap();
        });

        let output = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
            .args([
                "dgx",
                "qualify",
                "--manifest",
                manifest.to_str().unwrap(),
                "--signature",
                signature.to_str().unwrap(),
                "--dry-run",
                "--json",
                "--config-dir",
                config.to_str().unwrap(),
                "--idempotency-key",
                "qualification-cli-e2e",
            ])
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rendered: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(rendered["schema"], "sparkplane.qualification-plan/v1");
        assert_eq!(rendered["operation"], operation);
        server.join().unwrap();
    }
}
