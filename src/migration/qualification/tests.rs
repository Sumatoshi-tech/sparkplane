use super::*;
use axum::{body::Body, extract::State, http::Request, response::IntoResponse};
use std::sync::{Arc, Mutex};

struct Wire {
    requests: Mutex<Vec<(String, Value)>>,
    bad_usage: bool,
}

async fn respond(
    State(wire): State<Arc<Wire>>,
    request: Request<Body>,
) -> axum::response::Response {
    let path = request.uri().path().to_owned();
    if path.starts_with("/api/") || path.starts_with("/openai/") {
        assert_eq!(request.headers()["authorization"], "Bearer fixture-token");
    }
    let bytes = axum::body::to_bytes(request.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let body: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    wire.requests
        .lock()
        .unwrap()
        .push((path.clone(), body.clone()));
    let response = if path.ends_with("/status") {
        json!({"read_only":false,"degraded_reasons":[],"executor":"fixture"})
    } else if path.ends_with("/instances") {
        json!({"instances":[active().before]})
    } else if path == "/tokenize" {
        assert_eq!(body["model"], "model");
        json!({"tokens":[1,42]})
    } else if path == "/v1/completions" {
        assert_eq!(body["model"], "model");
        assert_eq!(body["prompt"], json!(vec![42; 8]));
        assert_eq!(body["max_tokens"], 8);
        assert_eq!(body["ignore_eos"], true);
        json!({"usage":{"prompt_tokens":8,"completion_tokens":if wire.bad_usage {7} else {8}}})
    } else if path == "/metrics" {
        return "vllm:num_requests_running{model=\"model\"} 0\nvllm:num_requests_waiting{model=\"model\"} 0\n".into_response();
    } else if body["reasoning_effort"] == "low" {
        json!({"choices":[{"message":{"reasoning_content":"Two plus two is four.","content":"4"}}]})
    } else if body["stream"] == true {
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["reasoning_effort"], "none");
        return ([("content-type", "text/event-stream")],
            "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\ndata: {\"choices\":[{\"finish_reason\":\"length\"}],\"usage\":{\"completion_tokens\":16}}\n\ndata: [DONE]\n\n").into_response();
    } else if body["tool_choice"] == "required" {
        assert_eq!(body["tools"][0]["function"]["name"], "migration_probe");
        json!({"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"call_fixture","type":"function","function":{"name":"migration_probe","arguments":"{\"value\":\"ok\"}"}}]}}]})
    } else {
        assert_eq!(path, "/openai/fixture/v1/chat/completions");
        assert_eq!(body["messages"][2]["tool_call_id"], "call_fixture");
        assert_eq!(body["messages"][2]["content"], "ok");
        json!({"choices":[{"message":{"content":"continued"}}]})
    };
    axum::Json(response).into_response()
}

struct Server {
    address: std::net::SocketAddr,
    certificate: String,
    handle: axum_server::Handle<std::net::SocketAddr>,
    thread: Option<std::thread::JoinHandle<()>>,
    wire: Arc<Wire>,
}
impl Server {
    fn start(tls: bool, bad_usage: bool) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
        let certificate = cert.pem();
        let pem = certificate.as_bytes().to_vec();
        let key = signing_key.serialize_pem().into_bytes();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = axum_server::Handle::new();
        let server_handle = handle.clone();
        let wire = Arc::new(Wire {
            requests: Mutex::new(vec![]),
            bad_usage,
        });
        let router = axum::Router::new()
            .fallback(respond)
            .with_state(wire.clone());
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    if tls {
                        let config = axum_server::tls_rustls::RustlsConfig::from_pem(pem, key)
                            .await
                            .unwrap();
                        axum_server::from_tcp_rustls(listener, config)
                            .unwrap()
                            .handle(server_handle)
                            .serve(router.into_make_service())
                            .await
                            .unwrap();
                    } else {
                        axum_server::from_tcp(listener)
                            .unwrap()
                            .handle(server_handle)
                            .serve(router.into_make_service())
                            .await
                            .unwrap();
                    }
                });
        });
        Self {
            address,
            certificate,
            handle,
            thread: Some(thread),
            wire,
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.handle.shutdown();
        self.thread.take().unwrap().join().unwrap();
    }
}

fn active() -> Active {
    serde_json::from_value(json!({"before":{
        "schema":"sy.spark.instance/v2","id":"fixture","name":"fixture","model_id":"model",
        "model":"huggingface:owner/model@immutable","model_commit":"a".repeat(40),
        "engine_id":"vllm","engine_fingerprint":"before","artifact_fingerprint":"before",
        "artifacts":{"schema":"sy.spark.model-artifacts/v2","format":"safetensors",
            "primary":{"path":"model.safetensors","bytes":8,"sha256":null},"auxiliary":[],
            "quantization":"NVFP4","capabilities":["text_generation","tool_calling"],"configured_alias":null},
        "objective":"agent","resources":{"image_bytes":1,"startup_peak_bytes":2,"steady_peak_bytes":1,"compile_cache_bytes":1},
        "context_window":16,"generation":8,"desired":"running","observed":"healthy",
        "endpoint":null,"healthy":true,"started_at":null,"last_failure":null},
        "container":{"container_id":"a".repeat(64),"instance_id":"fixture","generation":8,
            "image_digest":"image","engine_id":"vllm","engine_fingerprint":"before","artifact_fingerprint":"before",
            "model_repository":"owner/model","model_commit":"a".repeat(40)},
        "current_engine_fingerprint":"after","current_artifact_fingerprint":"after"})).unwrap()
}

#[test]
fn pinned_https_qualification_uses_authenticated_control_and_streaming_tool_routes() {
    let server = Server::start(true, false);
    let root = tempfile::tempdir().unwrap();
    for path in [
        "var/lib/sy-spark/ca",
        "var/lib/sparkplane/ca",
        "etc/sy",
        "etc/sparkplane",
    ] {
        std::fs::create_dir_all(root.path().join(path)).unwrap();
    }
    for path in [
        "var/lib/sy-spark/ca/ca-cert.pem",
        "var/lib/sparkplane/ca/ca-cert.pem",
    ] {
        std::fs::write(root.path().join(path), &server.certificate).unwrap();
    }
    for path in [
        "etc/sy/spark-bootstrap-admin.credential",
        "etc/sparkplane/bootstrap-admin.credential",
    ] {
        std::fs::write(root.path().join(path), "fixture-token").unwrap();
    }
    let mut plan = Plan {
        schema: "fixture".into(),
        host: String::new(),
        release: String::new(),
        executable: String::new(),
        uid: 996,
        gid: 996,
        listen: server.address,
        active: vec![active()],
        cache_keys: vec![],
        engines: BTreeMap::new(),
        agent_sha256: String::new(),
        executor_sha256: String::new(),
        inventory_sha256: String::new(),
        source_engines: BTreeMap::new(),
        model_catalog_sha256: String::new(),
        legacy_network: None,
    };
    for legacy in [true, false] {
        let gateway = Gateway::load(root.path(), &plan, legacy).unwrap();
        gateway.healthy(&plan, legacy).unwrap();
        gateway.reasoning(&plan.active[0]).unwrap();
        gateway.tools(&plan.active[0]).unwrap();
        let body = gateway.chat(&plan.active[0], "fixture");
        assert!(
            gateway
                .sample(&plan.active[0], &body, false)
                .unwrap()
                .is_finite()
        );
        assert_eq!(gateway.sample(&plan.active[0], &body, true).unwrap(), 0.0);
        plan.active[0].before.generation = 9;
        assert!(gateway.healthy(&plan, legacy).is_err());
        plan.active[0].before.generation = 8;
    }
    let paths = server.wire.requests.lock().unwrap();
    assert!(
        paths
            .iter()
            .any(|(path, _)| path == "/api/sy.spark/v1/status")
    );
    assert!(
        paths
            .iter()
            .any(|(path, _)| path == "/api/sparkplane/v1/status")
    );
    drop(paths);
    let wrong = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    std::fs::write(
        root.path().join("var/lib/sparkplane/ca/ca-cert.pem"),
        wrong.cert.pem(),
    )
    .unwrap();
    assert!(
        Gateway::load(root.path(), &plan, false)
            .unwrap()
            .healthy(&plan, false)
            .is_err()
    );
}

#[test]
fn maximum_context_requires_exact_usage_and_native_model_name() {
    for bad_usage in [false, true] {
        let server = Server::start(false, bad_usage);
        let base = format!("http://{}", server.address);
        let gateway = Gateway {
            client: native_client().unwrap(),
            base: base.clone(),
            token: String::new(),
        };
        assert_eq!(
            gateway.maximum_context(&active(), &base).is_ok(),
            !bad_usage
        );
        idle(&base).unwrap();
        assert_eq!(server.wire.requests.lock().unwrap().len(), 3);
    }
}
