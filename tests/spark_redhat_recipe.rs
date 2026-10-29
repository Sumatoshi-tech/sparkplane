#![cfg(feature = "appliance")]

use sparkplane::spark::{engine::EngineCatalog, model_catalog::ModelCatalog};
use std::fs;

#[test]
fn flash_next_defaults_to_redhat_and_preserves_explicit_alternatives() {
    let models = ModelCatalog::load("configs/sparkplane/models.toml".as_ref()).unwrap();
    for alias in [
        "qwen3.8:flash-next",
        "qwen3.8:flash-next-nvfp4",
        "qwen3.8:flash-next-redhat",
    ] {
        let model = models.resolve(alias).unwrap();
        assert_eq!(model.repository(), "RedHatAI/Qwen3.8-Flash-Next-NVFP4");
        assert_eq!(model.revision(), "c8f2fb1b9869f686b214782036123b10ff96d14a");
    }
    let radix = models.resolve("qwen3.8:flash-next-radixark").unwrap();
    assert_eq!(radix.repository(), "RadixArk/Qwen3.8-Flash-Next-NVFP4");
    assert_eq!(
        radix.artifacts().engine_profile.as_deref(),
        Some("qwen3.8-flash-next-nvfp4")
    );
    let abliterated = models.resolve("qwen3.8:flash-next-abliterated").unwrap();
    assert_eq!(
        abliterated.repository(),
        "DevelopingDad/Qwen3.8-Flash-Next-FP4-Local-Abliterated"
    );

    let files = fs::read_dir("configs/sparkplane/engines")
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "toml"))
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read_to_string(entry.path()).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let engines = EngineCatalog::parse_files(files).unwrap();
    let redhat = models.resolve("qwen3.8:flash-next").unwrap();
    let engine = engines.select(redhat.artifacts()).unwrap();
    assert_eq!(engine.config().id, "vllm-qwen38-redhat-arm64");
    assert_eq!(
        engines.select(radix.artifacts()).unwrap().config().id,
        "vllm-qwen38-mmap-arm64"
    );
    assert_eq!(
        engines.select(abliterated.artifacts()).unwrap().config().id,
        "vllm-qwen38-fp4-local-arm64"
    );
    assert_eq!(
        engine.config().image_repository,
        "sparkplane/vllm-qwen38-redhat"
    );
    assert_eq!(
        engine.config().image_digest,
        "sha256:5cfc1fe12892c42654abd337fae28be0dea8e67f575e669ba9c547c2b9fb0a62"
    );
    assert_eq!(engine.config().seccomp, "default");
    assert_eq!(engine.config().network, "sparkplane-internal");
    assert_eq!(engine.config().run_as_uid, 65534);
    let profile = engine
        .profile_for(Some("qwen4_exp"), redhat.artifacts())
        .unwrap();
    assert_eq!(profile.context_window, 262144);
    assert!(profile.startup_protocol_probe);
    assert!(profile.vision.is_some());
}

#[test]
fn redhat_config_patch_accepts_only_the_pinned_upstream_source() {
    use sha2::{Digest, Sha256};
    use std::process::Command;
    let patch = "configs/sparkplane/engines/patches/sha256-abfa66016b36ea4035777f9b02a5c9a6c3badc49d48e12c3320f016ba005efe4.py";
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(patch).unwrap())),
        "abfa66016b36ea4035777f9b02a5c9a6c3badc49d48e12c3320f016ba005efe4"
    );
    let original = fs::read("tests/fixtures/spark-redhat-config/config.py").unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&original)),
        "71230ef5e53e016416f54ae96b9f8b4e2c10a6311d8e6035ac063ac48fea2471"
    );
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.py");
    fs::write(&config, &original).unwrap();
    assert!(
        Command::new("python3")
            .args([patch])
            .arg(&config)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        Command::new("python3")
            .args(["-m", "py_compile"])
            .arg(&config)
            .output()
            .unwrap()
            .status
            .success()
    );
    let patched = fs::read(&config).unwrap();
    assert!(
        !Command::new("python3")
            .args([patch])
            .arg(&config)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(&config).unwrap(), patched);
    let mut changed = original;
    changed.extend_from_slice(b"\n# Different upstream revision.\n");
    fs::write(&config, &changed).unwrap();
    assert!(
        !Command::new("python3")
            .args([patch])
            .arg(&config)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(&config).unwrap(), changed);
}
