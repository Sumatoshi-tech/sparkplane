#![cfg(feature = "appliance")]
use sparkplane::migration::config::agent;
const AGENT: &str = include_str!("../configs/sparkplane/agent.toml");

#[test]
fn config_conversion_preserves_operator_resource_policy() {
    let legacy = AGENT
        .replace("sparkplane.agent/v1", "sy.spark.agent/v1")
        .replace("/etc/sparkplane/", "/etc/sy/spark/")
        .replace("/var/lib/sparkplane/", "/var/lib/sy-spark/")
        .replace("/run/sparkplane/", "/run/sy-spark/")
        .replace("/opt/sparkplane/", "/opt/sy-spark/");
    assert_eq!(
        agent(&legacy).unwrap().parse::<toml::Value>().unwrap(),
        AGENT.parse::<toml::Value>().unwrap()
    );
}

#[test]
fn executor_conversion_preserves_numeric_identity_and_hardware_fingerprint() {
    let current = include_str!("../configs/sparkplane/executor.toml").replace("996", "997");
    let legacy = current
        .replace("sparkplane.executor/v1", "sy.spark.executor/v1")
        .replace("/etc/sparkplane/agent.toml", "/etc/sy/spark-agent.toml")
        .replace("/etc/sparkplane/engines", "/etc/sy/spark/engines")
        .replace("/run/sparkplane/", "/run/sy-spark/");
    let migrated = sparkplane::migration::config::executor(&legacy).unwrap();
    assert_eq!(
        migrated.parse::<toml::Value>().unwrap(),
        current.parse::<toml::Value>().unwrap()
    );
}
