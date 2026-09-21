//! Convert owned path fields only; preserve operator policy values exactly.
use anyhow::{Context, Result, ensure};

fn field(table: &mut toml::Value, key: &str, from: &str, to: &str) -> Result<()> {
    ensure!(
        table.get(key).and_then(toml::Value::as_str) == Some(from),
        "unexpected legacy configuration {key}"
    );
    table[key] = to.into();
    Ok(())
}

pub fn agent(text: &str) -> Result<String> {
    let mut config: toml::Value = toml::from_str(text)?;
    field(
        &mut config,
        "schema",
        "sy.spark.agent/v1",
        "sparkplane.agent/v1",
    )?;
    field(
        &mut config,
        "engine_catalog",
        "/etc/sy/spark/engines",
        "/etc/sparkplane/engines",
    )?;
    field(
        &mut config,
        "model_catalog",
        "/etc/sy/spark/models.toml",
        "/etc/sparkplane/models.toml",
    )?;
    field(
        &mut config,
        "executor_socket",
        "/run/sy-spark/executor.sock",
        "/run/sparkplane/executor.sock",
    )?;
    let models = config.get_mut("models").context("missing model policy")?;
    field(
        models,
        "cache_root",
        "/var/lib/sy-spark/huggingface",
        "/var/lib/sparkplane/huggingface",
    )?;
    field(
        models,
        "fallback_executable",
        "/opt/sy-spark/hf-http-fallback/current/venv/bin/huggingface-cli",
        "/opt/sparkplane/hf-http-fallback/current/venv/bin/huggingface-cli",
    )?;
    let typed: crate::spark::agent::AgentConfig = config.clone().try_into()?;
    typed.resources.policy().map_err(anyhow::Error::msg)?;
    Ok(toml::to_string(&config)?)
}

pub fn executor(text: &str) -> Result<String> {
    let mut config: toml::Value = toml::from_str(text)?;
    field(
        &mut config,
        "schema",
        "sy.spark.executor/v1",
        "sparkplane.executor/v1",
    )?;
    field(
        &mut config,
        "socket",
        "/run/sy-spark/executor.sock",
        "/run/sparkplane/executor.sock",
    )?;
    field(
        &mut config,
        "engine_catalog",
        "/etc/sy/spark/engines",
        "/etc/sparkplane/engines",
    )?;
    field(
        &mut config,
        "resources_policy",
        "/etc/sy/spark-agent.toml",
        "/etc/sparkplane/agent.toml",
    )?;
    let _: crate::spark::executor::ExecutorConfig = config.clone().try_into()?;
    Ok(toml::to_string(&config)?)
}
