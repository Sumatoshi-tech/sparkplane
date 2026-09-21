#![cfg(feature = "appliance")]
use sparkplane::migration::verify_engine_transition;
const ENGINE: &str = include_str!("../configs/sparkplane/engines/vllm-qwen38-mmap.toml");

#[test]
fn optimized_engine_allows_only_namespace_changes() {
    let legacy = ENGINE
        .replace("sparkplane.engine/", "sy.spark.engine/")
        .replace("/var/lib/sparkplane/", "/var/lib/sy-spark/")
        .replace("sparkplane-internal", "sy-spark-internal")
        .replace("sparkplane/vllm", "sy-spark/vllm");
    assert!(verify_engine_transition(&legacy, ENGINE).is_ok());
}

#[test]
fn malformed_engine_is_an_error_not_a_panic() {
    assert!(verify_engine_transition("", ENGINE).is_err());
}

#[test]
fn runtime_context_image_and_sampling_changes_are_rejected() {
    let legacy = ENGINE
        .replace("sparkplane.engine/", "sy.spark.engine/")
        .replace("/var/lib/sparkplane/", "/var/lib/sy-spark/")
        .replace("sparkplane-internal", "sy-spark-internal")
        .replace("sparkplane/vllm", "sy-spark/vllm");
    for (from, to) in [
        ("262144", "131072"),
        ("MTP_DRAFT_VOCAB", "OTHER_VOCAB"),
        ("fa6008389ff17911", "aa6008389ff17911"),
        ("PIECEWISE", "NONE"),
    ] {
        assert!(
            verify_engine_transition(&legacy, &ENGINE.replace(from, to)).is_err(),
            "{from}"
        );
    }
}

#[test]
fn cache_namespace_transition_retains_the_exact_optimized_identity() {
    let policy = sparkplane::spark::engine::EnginePolicy::parse(ENGINE).unwrap();
    let old = "sha256:26d708967d17a7b261018701fffcce2f40f2efa98476905e1a7e365db4872de6";
    let keys = sparkplane::migration::cache_keys(
        &policy,
        "RadixArk/Qwen3.8-Flash-Next-NVFP4",
        "7b719225242aacd3dbd3f9407468c2ee9a9d2594",
        "qwen3.8-flash-next-nvfp4",
        old,
        old,
    )
    .unwrap();
    assert_eq!(
        keys.0,
        "sha256-047dd61595ca1fa5917da57798c54ee106660f37e2ed5347b8ad2ad0ccc98cab"
    );
    assert_ne!(keys.0, keys.1);
}

#[test]
fn cache_transition_rejects_unsafe_model_identity() {
    let policy = sparkplane::spark::engine::EnginePolicy::parse(ENGINE).unwrap();
    let digest = format!("sha256:{}", "a".repeat(64));
    assert!(
        sparkplane::migration::cache_keys(
            &policy,
            "../outside",
            &"a".repeat(40),
            "qwen3.8-flash-next-nvfp4",
            &digest,
            &digest
        )
        .is_err()
    );
}
