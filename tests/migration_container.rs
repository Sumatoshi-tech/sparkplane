#![cfg(feature = "appliance")]
use sparkplane::migration::container::LegacyContainer;

fn fixture() -> LegacyContainer {
    LegacyContainer {
        container_id: "a".repeat(64),
        instance_id: format!("i_{}", "b".repeat(32)),
        generation: 8,
        image_digest: format!("sha256:{}", "c".repeat(64)),
        engine_id: "engine".into(),
        engine_fingerprint: format!("sha256:{}", "d".repeat(64)),
        artifact_fingerprint: format!("sha256:{}", "e".repeat(64)),
        model_repository: "owner/model".into(),
        model_commit: "f".repeat(40),
    }
}

#[test]
fn cleanup_authority_is_bound_to_the_full_container_and_generation() {
    let expected = fixture();
    let inspect = serde_json::json!({"Id":expected.container_id, "Image":expected.image_digest,
        "Name":format!("/sy-spark-{}-g8", expected.instance_id), "Config":{"Labels":expected.labels()}});
    expected.verify(&inspect).unwrap();
    let mut wrong = inspect.clone();
    wrong["Id"] = "a".repeat(12).into();
    assert!(expected.verify(&wrong).is_err());
    wrong = inspect;
    wrong["Config"]["Labels"]["io.sy.spark.generation"] = "9".into();
    assert!(expected.verify(&wrong).is_err());
}
