use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};

#[test]
fn eco_mode_is_default_and_can_be_disabled() {
    let help = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
        .args(["fixture", "launch", "codex", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("--eco-mode"));
    assert!(help.contains("[default: max]"));
    let rejected = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
        .args(["fixture", "launch", "codex", "--eco-mode=invalid"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));
}

#[test]
fn embedded_rewrite_preserves_shell_and_structured_output() {
    for command in [
        "git status --porcelain",
        "git status > result",
        "git status | cat",
        "echo $(git status)",
        "cargo test --message-format=json",
        "printf '%s' '$secret'",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
            .args(["__eco", "rewrite", command])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty(), "{command}");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
        .args(["__eco", "rewrite", "git status"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("__eco exec")
    );
}

#[test]
fn hooks_compress_only_supported_calls_and_collect_private_metrics() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let cargo = bin.join("cargo");
    std::fs::write(&cargo,"#!/bin/sh\nprintf 'running 2 tests\\ntest first ... ok\\ntest second ... ok\\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\\n'\nexit 0\n").unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o700)).unwrap();
    let metrics = root.path().join("metrics");
    let binary = env!("CARGO_BIN_EXE_sparkplane");
    let mut hook = Command::new(binary)
        .args(["__eco", "hook", "codex"])
        .env("SPARKPLANE_ECO_REWRITE_ENABLED", "1")
        .env("SPARKPLANE_ECO_SESSION_DIR", &metrics)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    hook.stdin.take().unwrap().write_all(br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo test"}}"#).unwrap();
    let output = hook.wait_with_output().unwrap();
    let output: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rewritten = output["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    let filtered = Command::new("/bin/sh")
        .args(["-c", rewritten])
        .env("PATH", &bin)
        .env("SPARKPLANE_ECO_SESSION_DIR", &metrics)
        .output()
        .unwrap();
    assert!(filtered.status.success());
    let raw = Command::new(binary)
        .args(["__eco", "raw", "--", "cargo", "test"])
        .env("PATH", &bin)
        .output()
        .unwrap();
    assert!(raw.status.success());
    assert!(filtered.stdout.len() < raw.stdout.len());
    assert!(String::from_utf8_lossy(&filtered.stdout).contains("2 passed"));
    assert_eq!(
        std::fs::metadata(&metrics).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for file in std::fs::read_dir(&metrics).unwrap() {
        let path = file.unwrap().path();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let record = std::fs::read_to_string(path).unwrap();
        assert!(!record.contains("first"));
        assert!(!record.contains("cargo test"));
    }
    std::fs::write(&cargo,"#!/bin/sh\nprintf 'full stdout diagnostic\\n'\nprintf 'full stderr diagnostic\\n' >&2\nexit 7\n").unwrap();
    let failure = Command::new(binary)
        .args(["__eco", "exec", "--", "cargo", "test"])
        .env("PATH", &bin)
        .output()
        .unwrap();
    assert_eq!(failure.status.code(), Some(7));
    assert_eq!(failure.stdout, b"full stdout diagnostic\n");
    assert_eq!(failure.stderr, b"full stderr diagnostic\n");
}

#[test]
fn disabled_hooks_emit_no_rewrite() {
    let output = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
        .args(["__eco", "rewrite", "git status"])
        .env("SPARKPLANE_ECO_REWRITE_ENABLED", "0")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn economics_json_is_a_retained_offline_report() {
    use sparkplane::spark::economics::{self, Compression, Report};
    let root = tempfile::tempdir().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let mut report = Report {
        schema: economics::SCHEMA.into(),
        session_id: ulid::Ulid::new().to_string(),
        host: "offline".into(),
        model: "fixture".into(),
        integration: "codex".into(),
        eco_mode: "none".into(),
        started_at: now.clone(),
        finished_at: Some(now),
        exit_code: Some(7),
        inference: None,
        compression: Compression {
            status: "disabled".into(),
            ..Default::default()
        },
        catalog: economics::catalog().unwrap(),
        comparisons: Vec::new(),
    };
    report.calculate();
    economics::save(root.path(), &report).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
        .args(["offline", "economics", "--json", "--config-dir"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["session_id"], report.session_id);
    assert_eq!(value["exit_code"], 7);
    assert!(value["inference"].is_null());
    assert_eq!(value["comparisons"], serde_json::json!([]));
}

#[test]
fn wrapper_forwards_termination_and_preserves_signal_exit_status() {
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_sparkplane"))
        .args([
            "__eco",
            "exec",
            "--",
            "sh",
            "-c",
            "printf ready > ready; exec sleep 30",
        ])
        .current_dir(root.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !root.path().join("ready").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.id() as i32),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(143));
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("eco child did not terminate");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
