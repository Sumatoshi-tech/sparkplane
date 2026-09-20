#[test]
fn optimized_runtime_keeps_graphs_without_global_eager_module_loading() {
    let profile: toml::Value = toml::from_str(include_str!(
        "../configs/sparkplane/engines/vllm-qwen38-mmap.toml"
    ))
    .unwrap();
    assert!(
        !profile["environment"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str() == Some("CUDA_MODULE_LOADING=EAGER"))
    );
    let args = profile["profiles"][0]["arguments"].as_array().unwrap();
    assert!(
        args.iter()
            .any(|value| value.as_str() == Some("-cc.cudagraph_mode=PIECEWISE"))
    );
    assert!(
        !args
            .iter()
            .any(|value| value.as_str() == Some("--enforce-eager"))
    );
}

#[test]
fn sampled_speculation_is_warmed_before_the_engine_accepts_traffic() {
    let image = include_str!("../configs/sparkplane/engines/vllm-qwen38-mmap.Dockerfile");
    assert!(image.contains("RUN python3 /opt/llm/patches/qwen38_sampling.py"));
    assert!(
        image.contains("COPY runtime/qwen38_sampling.py ${SITE_PACKAGES}/sy_qwen38_sampling.py")
    );
}

#[test]
fn mixed_prefill_and_decode_metadata_are_patched_and_exercised() {
    let image = include_str!("../configs/sparkplane/engines/vllm-qwen38-mmap.Dockerfile");
    assert!(image.contains("RUN python3 /opt/llm/patches/qwen38_cpu_metadata.py"));
    let checks = include_str!("../configs/sparkplane/engines/checks/qwen38_optimization.py");
    assert!(checks.contains("check_mixed_metadata(prefill_tokens=8187, spec_tokens=4)"));
}

#[test]
fn optimized_embedding_gathers_always_use_the_worker_pool() {
    let profile: toml::Value = toml::from_str(include_str!(
        "../configs/sparkplane/engines/vllm-qwen38-mmap.toml"
    ))
    .unwrap();
    assert!(
        profile["environment"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| { value.as_str() == Some("VLLM_PLE_MMAP_FAST_ROWS=0") })
    );
}

#[test]
fn runtime_assets_are_exercised_as_the_serving_user() {
    let image = include_str!("../configs/sparkplane/engines/vllm-qwen38-mmap.Dockerfile");
    assert!(
        image.contains("USER 65534\nRUN HOME=/tmp python3 /opt/llm/checks/qwen38_optimization.py")
    );
}

#[test]
fn gb10_linear_attention_uses_the_correct_shared_memory_limit() {
    let image = include_str!("../configs/sparkplane/engines/vllm-qwen38-mmap.Dockerfile");
    assert!(image.contains("DEFAULT = 101376"));
    assert!(image.contains("for num_warps in [2]"));
}

#[test]
fn three_token_speculation_preserves_native_context_and_kv_precision() {
    let policy: toml::Value = toml::from_str(include_str!(
        "../configs/sparkplane/engines/vllm-qwen38-mmap.toml"
    ))
    .unwrap();
    let args = policy["profiles"][0]["arguments"].as_array().unwrap();
    assert!(
        args.iter()
            .any(|v| v.as_str() == Some("{\"method\":\"mtp\",\"num_speculative_tokens\":3}"))
    );
    assert_eq!(
        policy["profiles"][0]["context_window"].as_integer(),
        Some(262144)
    );
    assert!(
        args.windows(2)
            .any(|v| v[0].as_str() == Some("--kv-cache-dtype") && v[1].as_str() == Some("auto"))
    );
}

#[test]
fn enabled_prefix_reuse_selects_deterministic_attention() {
    let policy: toml::Value = toml::from_str(include_str!(
        "../configs/sparkplane/engines/vllm-qwen38-mmap.toml"
    ))
    .unwrap();
    assert!(
        policy["environment"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("VLLM_QSA_DET_TOPK=1"))
    );
    assert!(
        policy["profiles"][0]["arguments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("--enable-prefix-caching"))
    );
}

#[test]
fn deterministic_attention_is_wired_into_the_runtime() {
    let image = include_str!("../configs/sparkplane/engines/vllm-qwen38-mmap.Dockerfile");
    assert!(image.contains("python3 /opt/llm/qsadet_patch.py"));
}

#[test]
fn deterministic_attention_kernel_is_built_for_spark() {
    let image = include_str!("../configs/sparkplane/engines/vllm-qwen38-mmap.Dockerfile");
    assert!(image.contains("DET_ARCH=121a"));
    assert!(image.contains("python3 build_det.py"));
}

#[test]
fn prefix_cache_image_repairs_recurrent_state_restoration() {
    let image = include_str!("../configs/sparkplane/engines/vllm-qwen38-mmap.Dockerfile");
    assert!(image.contains("python3 /opt/llm/patch_mamba_block_size.py"));
    assert!(image.contains("${RECIPE}/src/mamba_utils_guarded.py"));
}

#[test]
fn reduced_vocabulary_is_installed_with_its_runtime_hook() {
    let image = include_str!("../configs/sparkplane/engines/vllm-qwen38-mmap.Dockerfile");
    assert!(image.contains("python3 /opt/llm/patch_mtp_draft_vocab.py"));
    assert!(image.contains("${RECIPE}/src/draft_vocab_65536.npy /opt/llm/draft_vocab_65536.npy"));
}

#[test]
fn optimized_image_runs_embedding_behavior_tests_during_build() {
    let image = include_str!("../configs/sparkplane/engines/vllm-qwen38-mmap.Dockerfile");
    assert!(image.contains("python3 /opt/llm/checks/test_ple_mmap_cpu.py"));
}

#[test]
fn draft_projection_uses_the_baked_in_reduced_vocabulary() {
    let policy: toml::Value = toml::from_str(include_str!(
        "../configs/sparkplane/engines/vllm-qwen38-mmap.toml"
    ))
    .unwrap();
    assert!(
        policy["environment"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| {
                value.as_str() == Some("VLLM_MTP_DRAFT_VOCAB=/opt/llm/draft_vocab_65536.npy")
            })
    );
}
