//! Fixed, content-free acceptance probes through pinned HTTPS and the exact engine.
use super::{
    appliance::{Active, Plan},
    container::Namespace,
};
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read},
    path::Path,
    time::{Duration, Instant},
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub decode_tokens_per_second: BTreeMap<String, Vec<f64>>,
}

pub fn compare(before: &Evidence, after: &Evidence) -> Result<()> {
    ensure!(
        before
            .decode_tokens_per_second
            .keys()
            .eq(after.decode_tokens_per_second.keys()),
        "qualification instance set changed"
    );
    for (id, first) in &before.decode_tokens_per_second {
        let second = &after.decode_tokens_per_second[id];
        let median = |values: &[f64]| -> Result<f64> {
            ensure!(
                values.len() == 3 && values.iter().all(|v| v.is_finite() && *v > 0.0),
                "invalid matched performance samples"
            );
            let mut sorted = values.to_vec();
            sorted.sort_by(f64::total_cmp);
            Ok(sorted[1])
        };
        ensure!(
            median(second)? >= median(first)? * 0.95,
            "matched decode throughput regressed more than five percent for {id}"
        );
    }
    Ok(())
}

pub struct Gateway {
    client: Client,
    base: String,
    token: String,
}

impl Gateway {
    pub fn load(root: &Path, plan: &Plan, legacy: bool) -> Result<Self> {
        let (ca, token) = if legacy {
            (
                "var/lib/sy-spark/ca/ca-cert.pem",
                "etc/sy/spark-bootstrap-admin.credential",
            )
        } else {
            (
                "var/lib/sparkplane/ca/ca-cert.pem",
                "etc/sparkplane/bootstrap-admin.credential",
            )
        };
        let ca = reqwest::Certificate::from_pem(&super::release::read(&root.join(ca), 65536)?)?;
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .https_only(true)
                .tls_built_in_root_certs(false)
                .add_root_certificate(ca)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(1800))
                .build()?,
            base: format!("https://{}", plan.listen),
            token: String::from_utf8(super::release::read(&root.join(token), 8192)?)?
                .trim()
                .into(),
        })
    }

    pub fn healthy(&self, plan: &Plan, legacy: bool) -> Result<()> {
        let namespace = if legacy { "sy.spark" } else { "sparkplane" };
        let status = json_response(
            self.client
                .get(format!("{}/api/{namespace}/v1/status", self.base))
                .bearer_auth(&self.token)
                .timeout(Duration::from_secs(5))
                .send()?,
        )?;
        ensure!(
            status["read_only"] == false
                && status["degraded_reasons"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
                && status["executor"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s != "unavailable"),
            "control plane is degraded"
        );
        let value = json_response(
            self.client
                .get(format!("{}/api/{namespace}/v1/instances", self.base))
                .bearer_auth(&self.token)
                .timeout(Duration::from_secs(5))
                .send()?,
        )?;
        let instances = value["instances"]
            .as_array()
            .context("instance inventory missing")?;
        for active in &plan.active {
            let row = instances
                .iter()
                .find(|row| row["id"] == active.before.id)
                .context("migrated instance missing")?;
            ensure!(
                row["healthy"] == true
                    && row["generation"] == active.before.generation
                    && row["context_window"] == active.before.context_window
                    && row["model"] == active.before.model
                    && row["restart_suppressed"] == false,
                "instance is not ready with its original identity and context"
            );
        }
        Ok(())
    }

    pub fn wait_healthy(&self, plan: &Plan, legacy: bool) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(1800);
        loop {
            if self.healthy(plan, legacy).is_ok() {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "appliance readiness deadline exceeded; traffic remains fenced"
            );
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    fn chat(&self, active: &Active, prompt: &str) -> Value {
        json!({"model":active.before.model,"messages":[{"role":"user","content":prompt}],"max_tokens":256,
            "temperature":0,"reasoning_effort":"none","stream":true,"stream_options":{"include_usage":true}})
    }

    fn sample(&self, active: &Active, body: &Value, cancel: bool) -> Result<f64> {
        let response = self
            .client
            .post(format!(
                "{}/openai/{}/v1/chat/completions",
                self.base, active.before.name
            ))
            .bearer_auth(&self.token)
            .json(body)
            .send()?;
        ensure!(
            response.status().is_success(),
            "gateway qualification request was rejected"
        );
        stream_sample(response, cancel)
    }

    pub fn qualify(
        &self,
        plan: &Plan,
        containers: &[Value],
        namespace: Namespace,
    ) -> Result<Evidence> {
        let mut evidence = Evidence {
            decode_tokens_per_second: BTreeMap::new(),
        };
        for active in &plan.active {
            let policy = plan
                .engines
                .values()
                .map(|text| {
                    crate::spark::engine::EnginePolicy::parse(text).map_err(anyhow::Error::msg)
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .find(|p| p.config().id == active.before.engine_id)
                .context("qualification engine missing")?;
            let container = containers
                .iter()
                .find(|value| {
                    value
                        .pointer(&format!("/Config/Labels/{}.instance", namespace.prefix()))
                        .and_then(Value::as_str)
                        == Some(&active.before.id)
                })
                .context("qualification container missing")?;
            if namespace == Namespace::Legacy {
                active.container.verify(container)?;
            } else {
                active.current(container)?;
            }
            let network = if namespace == Namespace::Legacy {
                "sy-spark-internal"
            } else {
                "sparkplane-internal"
            };
            let address: std::net::IpAddr = container
                .pointer(&format!("/NetworkSettings/Networks/{network}/IPAddress"))
                .and_then(Value::as_str)
                .context("engine private address missing")?
                .parse()?;
            ensure!(
                match address {
                    std::net::IpAddr::V4(ip) => ip.is_private(),
                    std::net::IpAddr::V6(ip) => ip.is_unique_local(),
                },
                "engine qualification address is not private"
            );
            let native = format!(
                "http://{}",
                std::net::SocketAddr::new(address, policy.config().port)
            );
            self.maximum_context(active, &native)?;
            self.reasoning(active)?;
            self.tools(active)?;
            let body = self.chat(active, "Write a detailed Rust implementation of a bounded queue, explaining invariants and cancellation. Continue until the output limit.");
            self.sample(active, &body, false)?; // One matched warmup in each generation.
            let samples = (0..3)
                .map(|_| self.sample(active, &body, false))
                .collect::<Result<Vec<_>>>()?;
            evidence
                .decode_tokens_per_second
                .insert(active.before.id.clone(), samples);
            std::thread::scope(|scope| -> Result<()> {
                let first = scope.spawn(|| self.sample(active, &body, false));
                let second = scope.spawn(|| self.sample(active, &body, false));
                first
                    .join()
                    .map_err(|_| anyhow::anyhow!("concurrent probe panicked"))??;
                second
                    .join()
                    .map_err(|_| anyhow::anyhow!("concurrent probe panicked"))??;
                Ok(())
            })?;
            self.sample(active, &body, true)?;
            idle(&native)?;
        }
        Ok(evidence)
    }

    fn reasoning(&self, active: &Active) -> Result<()> {
        let result = json_response(
            self.client
                .post(format!(
                    "{}/openai/{}/v1/chat/completions",
                    self.base, active.before.name
                ))
                .bearer_auth(&self.token)
                .json(&json!({"model":active.before.model,
                "messages":[{"role":"user","content":"What is 2+2? Answer with just the integer."}],
                "max_tokens":1024,"temperature":0,"reasoning_effort":"low"}))
                .send()?,
        )?;
        let message = &result["choices"][0]["message"];
        ensure!(
            message["reasoning_content"]
                .as_str()
                .or_else(|| message["reasoning"].as_str())
                .is_some_and(|s| !s.trim().is_empty())
                && message["content"].as_str().is_some_and(|s| s.contains('4')),
            "reasoning probe must return separate reasoning and answer content"
        );
        Ok(())
    }

    fn tools(&self, active: &Active) -> Result<()> {
        let url = format!(
            "{}/openai/{}/v1/chat/completions",
            self.base, active.before.name
        );
        let tool = json!({"type":"function","function":{"name":"migration_probe","description":"Check tool continuation","parameters":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}}});
        let body = json!({"model":active.before.model,"messages":[{"role":"user","content":"Call migration_probe with value ok."}],"tools":[tool],"tool_choice":"required","max_tokens":256,"temperature":0,"reasoning_effort":"none"});
        let result = json_response(
            self.client
                .post(&url)
                .bearer_auth(&self.token)
                .json(&body)
                .send()?,
        )?;
        let message = &result["choices"][0]["message"];
        let calls = message["tool_calls"]
            .as_array()
            .filter(|v| v.len() == 1)
            .context("tool call missing")?;
        ensure!(
            calls[0]["function"]["name"] == "migration_probe",
            "tool identity mismatch"
        );
        let arguments: Value = serde_json::from_str(
            calls[0]["function"]["arguments"]
                .as_str()
                .context("tool arguments missing")?,
        )?;
        ensure!(arguments["value"] == "ok", "tool argument mismatch");
        let follow = json!({"model":active.before.model,"messages":[body["messages"][0],message,{"role":"tool","tool_call_id":calls[0]["id"],"content":"ok"}],"max_tokens":64,"reasoning_effort":"none"});
        let result = json_response(
            self.client
                .post(url)
                .bearer_auth(&self.token)
                .json(&follow)
                .send()?,
        )?;
        ensure!(
            result["choices"][0]["message"]["content"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty()),
            "tool continuation returned no content"
        );
        Ok(())
    }

    fn maximum_context(&self, active: &Active, native: &str) -> Result<()> {
        let context = active.before.context_window;
        ensure!(
            (16..=1_048_576).contains(&context),
            "unsupported qualification context"
        );
        let client = native_client()?;
        let model = active
            .container
            .model_repository
            .rsplit('/')
            .next()
            .context("native model identity missing")?;
        let tokens = json_response(
            client
                .post(format!("{native}/tokenize"))
                .json(&json!({"model":model,"prompt":" migration"}))
                .send()?,
        )?;
        let token = tokens["tokens"]
            .as_array()
            .and_then(|tokens| tokens.last())
            .and_then(Value::as_u64)
            .context("native tokenizer returned no token")?;
        let prompt = vec![token; (context - 8) as usize];
        let response = json_response(client.post(format!("{native}/v1/completions")).json(&json!({"model":model,"prompt":prompt,"max_tokens":8,"ignore_eos":true,"temperature":0})).send()?)?;
        ensure!(
            response["usage"]["prompt_tokens"] == context - 8
                && response["usage"]["completion_tokens"] == 8,
            "maximum-context inference did not consume the full configured window"
        );
        Ok(())
    }
}

fn native_client() -> Result<Client> {
    Ok(Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(1800))
        .build()?)
}

fn json_response(response: reqwest::blocking::Response) -> Result<Value> {
    ensure!(
        response.status().is_success(),
        "qualification endpoint rejected the request"
    );
    let mut bytes = Vec::new();
    response.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "qualification response exceeds limit"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

fn idle(native: &str) -> Result<()> {
    let client = native_client()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let response = client
            .get(format!("{native}/metrics"))
            .timeout(Duration::from_secs(5))
            .send()?;
        ensure!(
            response.status().is_success(),
            "engine cancellation metrics unavailable"
        );
        let mut metrics = String::new();
        response.take(1024 * 1024).read_to_string(&mut metrics)?;
        let rows: Vec<_> = metrics
            .lines()
            .filter(|s| {
                s.starts_with("vllm:num_requests_running{")
                    || s.starts_with("vllm:num_requests_waiting{")
            })
            .collect();
        if rows.len() >= 2
            && rows.iter().all(|s| {
                s.split_whitespace()
                    .last()
                    .and_then(|n| n.parse::<f64>().ok())
                    == Some(0.0)
            })
        {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "cancelled inference did not drain"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn stream_sample(reader: impl Read, cancel: bool) -> Result<f64> {
    let mut reader = BufReader::new(reader.take(4 * 1024 * 1024));
    let mut first = None;
    let mut tokens = None;
    let mut finished = false;
    let mut done = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let Some(data) = line.trim().strip_prefix("data: ") else {
            continue;
        };
        if data == "[DONE]" {
            done = true;
            break;
        }
        let value: Value = serde_json::from_str(data)?;
        ensure!(
            value.get("error").is_none(),
            "qualification stream returned an error"
        );
        if value["choices"][0]["delta"]["content"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
        {
            first.get_or_insert_with(Instant::now);
            if cancel {
                return Ok(0.0);
            }
        }
        finished |= value["choices"][0]["finish_reason"].as_str().is_some();
        if let Some(usage) = value["usage"]["completion_tokens"].as_u64() {
            tokens = Some(usage);
        }
    }
    ensure!(finished && done, "incomplete qualification stream");
    let count = tokens
        .filter(|n| *n > 1)
        .context("qualification usage missing")?;
    let elapsed = first
        .context("qualification stream has no content")?
        .elapsed()
        .as_secs_f64();
    ensure!(elapsed > 0.0, "invalid qualification timing");
    Ok((count - 1) as f64 / elapsed)
}

#[cfg(test)]
mod tests;
