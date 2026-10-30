#![cfg(feature = "appliance")]

use sparkplane::spark::{engine::EngineCatalog, model_catalog::ModelCatalog};
use std::{fs, process::Command};

#[test]
fn fp4_checkpoint_reader_preserves_bytes_strides_and_failure_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = "tests/fixtures/spark-fp4-loader";
    for name in ["_bounce.c", "_batch.c", "_direct.c"] {
        fs::copy(format!("{fixture}/{name}"), directory.path().join(name)).unwrap();
    }
    let dockerfile =
        fs::read_to_string("configs/sparkplane/engines/vllm-qwen38-fp4-local.Dockerfile").unwrap();
    let patch = dockerfile
        .lines()
        .find_map(|line| line.strip_prefix("COPY patches/"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    let patch = format!("configs/sparkplane/engines/patches/{patch}");
    assert!(
        Command::new("python3")
            .arg(&patch)
            .arg(directory.path())
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("python3")
            .arg("configs/sparkplane/engines/checks/qwen38_fp4_checkpoint.py")
            .arg(directory.path())
            .status()
            .unwrap()
            .success()
    );
    // The source guard refuses repeated edits or a different upstream reader.
    assert!(
        !Command::new("python3")
            .arg(&patch)
            .arg(directory.path())
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn fp4_local_abliterated_selects_its_runtime_without_changing_stock_flash_next() {
    let models = ModelCatalog::load("configs/sparkplane/models.toml".as_ref()).unwrap();
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
    let stock = models.resolve("qwen3.8:flash-next-radixark").unwrap();
    let abliterated = models.resolve("qwen3.8:flash-next-abliterated").unwrap();
    let stock_engine = engines.select(stock.artifacts()).unwrap();
    let engine = engines.select(abliterated.artifacts()).unwrap();
    assert_ne!(stock_engine.config().id, engine.config().id);
    assert_eq!(stock_engine.config().id, "vllm-qwen38-mmap-arm64");
    assert_eq!(abliterated.artifacts().auxiliary.len(), 21);
    assert!(abliterated.artifacts().auxiliary.iter().all(|file| {
        file.path.starts_with("tensors/") && file.sha256.is_some() && file.bytes > 0
    }));

    let config = engine.config();
    assert_eq!(config.seccomp, "default");
    assert_eq!(config.network, "sparkplane-internal");
    assert_eq!(config.run_as_uid, 65534);
    assert!(
        config
            .environment
            .iter()
            .any(|entry| entry == "VLLM_PLE_TABLE_MEMORY=disk")
    );
    assert!(
        config
            .environment
            .iter()
            .any(|entry| entry == "B12X_CHECKPOINT_PREAD=1")
    );
    assert!(
        config
            .environment
            .iter()
            .any(|entry| entry == "B12X_DISK_BACKEND=pread")
    );
    let profile = engine
        .profile_for(Some("qwen4_exp"), abliterated.artifacts())
        .unwrap();
    assert_eq!(profile.context_window, 262144);
    assert!(profile.startup_protocol_probe);
    for pair in [
        ["--load-format", "b12x"],
        ["--quantization", "modelopt_mixed"],
        ["--kv-cache-dtype", "fp8"],
        ["--tool-call-parser", "qwen3_xml"],
    ] {
        assert!(profile.arguments.windows(2).any(|args| args == pair));
    }
    assert!(profile.vision.is_some());
    assert_eq!(profile.sampling.temperature, Some(1.0));
}

#[test]
fn fp4_disk_ple_preserves_rows_and_drains_failed_reads() {
    let directory = tempfile::tempdir().unwrap();
    let loader = directory.path().join("loader");
    fs::create_dir(&loader).unwrap();
    let sequence = directory.path().join("sequence/_shared");
    fs::create_dir_all(&sequence).unwrap();
    fs::copy(
        "tests/fixtures/spark-fp4-loader/disk_table.py",
        sequence.join("disk_table.py"),
    )
    .unwrap();
    for name in ["_ple_reader.c", "_row_plan.h"] {
        fs::copy(
            format!("tests/fixtures/spark-fp4-loader/{name}"),
            loader.join(name),
        )
        .unwrap();
    }
    let dockerfile =
        fs::read_to_string("configs/sparkplane/engines/vllm-qwen38-fp4-local.Dockerfile").unwrap();
    let patch = dockerfile
        .lines()
        .filter_map(|line| line.strip_prefix("COPY patches/"))
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    let patch = format!("configs/sparkplane/engines/patches/{patch}");
    assert!(
        Command::new("python3")
            .arg(&patch)
            .arg(&loader)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("python3")
            .arg("configs/sparkplane/engines/checks/qwen38_fp4_ple.py")
            .arg(&loader)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        !Command::new("python3")
            .arg(&patch)
            .arg(&loader)
            .output()
            .unwrap()
            .status
            .success()
    );
}
