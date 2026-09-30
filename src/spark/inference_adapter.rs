//! Invocation-local authenticated loopback adapter. Only fixed inference routes
//! reach the pinned appliance; images are projected before the network upload.
use super::{
    client::{self, ClientError},
    image_history::{self, Protocol},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bytes::Bytes;
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::{
    Request, Response,
    body::{Frame, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    net::TcpListener,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};
use tokio::sync::{Semaphore, oneshot};

type Body = UnsyncBoxBody<Bytes, io::Error>;
const ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const IMAGE_BYTES: usize = 16 * 1024 * 1024;

struct Archive {
    root: PathBuf,
}
impl Archive {
    fn save(&self, id: &str, part: &Value, protocol: Protocol) -> Option<PathBuf> {
        let (media, data) = if protocol == Protocol::Anthropic {
            if part["source"]["type"] != "base64" {
                return None;
            }
            (
                part["source"]["media_type"].as_str()?,
                part["source"]["data"].as_str()?,
            )
        } else {
            let uri = if protocol == Protocol::Responses {
                part["image_url"].as_str()?
            } else {
                part["image_url"]["url"].as_str()?
            };
            uri.strip_prefix("data:")?.split_once(";base64,")?
        };
        let extension = match media {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => return None,
        };
        if data.len() > IMAGE_BYTES.div_ceil(3) * 4 {
            return None;
        }
        let path = self.root.join(format!("{id}.{extension}"));
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.root.join("archive.lock"))
            .ok()?;
        let _lock =
            nix::fcntl::Flock::lock(lock, nix::fcntl::FlockArg::LockExclusiveNonblock).ok()?;
        if let Ok(meta) = fs::symlink_metadata(&path) {
            let expected_bytes = (data.len() / 4 * 3)
                .checked_sub(data.bytes().rev().take_while(|byte| *byte == b'=').count())?;
            if !meta.is_file()
                || meta.mode() & 0o077 != 0
                || meta.uid() != rustix::process::geteuid().as_raw()
                || meta.len() != expected_bytes as u64
            {
                return None;
            }
            let file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)
                .ok()?;
            let _ = file.set_modified(std::time::SystemTime::now());
            return Some(path);
        }
        let bytes = BASE64.decode(data).ok()?;
        if bytes.is_empty() || bytes.len() > IMAGE_BYTES {
            return None;
        }
        if !match media {
            "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "image/jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
            "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
            _ => false,
        } {
            return None;
        }
        let mut entries = Vec::new();
        let mut total = 0;
        for (index, entry) in fs::read_dir(&self.root).ok()?.take(4098).enumerate() {
            if index > 4096 {
                return None;
            }
            let entry = entry.ok()?;
            let name = entry.file_name();
            let name = name.to_str()?;
            if name.starts_with(".staging-") && name[9..].parse::<uuid::Uuid>().is_ok() {
                fs::remove_file(entry.path()).ok()?;
                continue;
            }
            let Some((stem, ext)) = name.split_once('.') else {
                continue;
            };
            if stem.len() != 64
                || !stem.bytes().all(|b| b.is_ascii_hexdigit())
                || !matches!(ext, "png" | "jpg" | "webp")
            {
                continue;
            }
            let meta = fs::symlink_metadata(entry.path()).ok()?;
            if !meta.is_file() {
                return None;
            }
            total += meta.len();
            entries.push((meta.modified().ok()?, meta.len(), entry.path()));
        }
        entries.sort_by_key(|entry| entry.0);
        let mut remaining = entries.len();
        for (_, len, old) in &entries {
            let expired = fs::symlink_metadata(old)
                .ok()?
                .modified()
                .ok()?
                .elapsed()
                .ok()?
                .as_secs()
                > 90 * 86400;
            if !expired && total + bytes.len() as u64 <= ARCHIVE_BYTES && remaining < 4096 {
                break;
            }
            fs::remove_file(old).ok()?;
            total = total.saturating_sub(*len);
            remaining -= 1;
        }
        if total + bytes.len() as u64 > ARCHIVE_BYTES {
            return None;
        }
        let staging = self.root.join(format!(".staging-{}", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&staging)
            .ok()?;
        if file
            .write_all(&bytes)
            .and_then(|_| file.sync_data())
            .and_then(|_| fs::rename(&staging, &path))
            .is_err()
        {
            let _ = fs::remove_file(&staging);
            return None;
        }
        Some(path)
    }
}
struct State {
    http: reqwest::Client,
    base: reqwest::Url,
    instance: String,
    remote_token: String,
    local_token: String,
    archive: Arc<Mutex<Archive>>,
    slots: Arc<Semaphore>,
}
pub struct Adapter {
    pub base_url: String,
    token: String,
    stop: Option<oneshot::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Adapter {
    pub fn start(
        config_dir: &Path,
        host: &str,
        instance: &str,
        remote_token: &str,
    ) -> Result<Self, ClientError> {
        let (http, base) = client::inference_transport(config_dir, host)?;
        let root = config_dir.join("spark").join("image-archive");
        super::economics::private_dir(&root)
            .map_err(|_| failure("private image archive is unavailable"))?;
        Self::start_with(http, base, instance, remote_token, root)
    }
    fn start_with(
        http: reqwest::Client,
        base: reqwest::Url,
        instance: &str,
        remote_token: &str,
        root: PathBuf,
    ) -> Result<Self, ClientError> {
        client::validate_instance_reference(instance)?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|_| failure("cannot bind the private inference adapter"))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| failure("cannot configure inference adapter"))?;
        let address = listener
            .local_addr()
            .map_err(|_| failure("cannot identify inference adapter"))?;
        let token = format!(
            "sparkplane_local_{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let state = Arc::new(State {
            http,
            base,
            instance: instance.into(),
            remote_token: remote_token.into(),
            local_token: token.clone(),
            archive: Arc::new(Mutex::new(Archive { root })),
            slots: Arc::new(Semaphore::new(2)),
        });
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| failure("cannot start inference adapter runtime"))?;
        let (stop, mut stopping) = oneshot::channel();
        let worker = thread::spawn(move || {
            runtime.block_on(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return; };
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut stopping => break,
                    Some(_) = connections.join_next(), if !connections.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break; };
                        if connections.len() >= 32 { continue; }
                        let state = state.clone();
                        connections.spawn(async move {
                            let service = service_fn(move |request| {
                                let state = state.clone();
                                async move { Ok::<_, std::convert::Infallible>(handle(request, state).await) }
                            });
                            let _ = http1::Builder::new().max_buf_size(16 * 1024).serve_connection(TokioIo::new(stream), service).await;
                        });
                    }
                }
            }
            connections.abort_all();
        })
        });
        Ok(Self {
            base_url: format!("http://{address}"),
            token,
            stop: Some(stop),
            worker: Some(worker),
        })
    }
    pub fn token(&self) -> &str {
        &self.token
    }
}
impl Drop for Adapter {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn failure(message: &'static str) -> ClientError {
    ClientError {
        code: super::EXIT_INTERNAL,
        message: message.into(),
    }
}
fn error(status: u16, message: &'static str) -> Response<Body> {
    let bytes = serde_json::to_vec(&json!({"type":"error","error":{"type":"invalid_request_error","code":if status == 413 { "request_too_large" } else if message.starts_with("history text exceeds") { "context_length_exceeded" } else { "invalid_request_error" },"message":message}})).expect("static error");
    let mut builder = Response::builder()
        .status(status)
        .header("content-type", "application/json");
    if status == 429 {
        builder = builder.header("retry-after", "1");
    }
    builder
        .body(
            Full::new(Bytes::from(bytes))
                .map_err(|never| match never {})
                .boxed_unsync(),
        )
        .expect("static response")
}
struct ChunkReader {
    receiver: mpsc::Receiver<Bytes>,
    current: std::io::Cursor<Bytes>,
}
impl Read for ChunkReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            let n = self.current.read(out)?;
            if n != 0 {
                return Ok(n);
            }
            match self.receiver.recv_timeout(Duration::from_secs(30)) {
                Ok(bytes) => self.current = std::io::Cursor::new(bytes),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
                Err(_) => return Err(io::Error::other("upload timed out")),
            }
        }
    }
}
async fn handle(request: Request<Incoming>, state: Arc<State>) -> Response<Body> {
    if request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        != Some(&format!("Bearer {}", state.local_token))
    {
        return error(401, "private adapter authentication failed");
    }
    let path = request.uri().path();
    if (request.uri().query().is_some()
        && !(path.starts_with("/anthropic/") && request.uri().query() == Some("beta=true")))
        || request.headers().contains_key("origin")
    {
        return error(400, "adapter route is unsupported");
    }
    let (route, protocol) = match (request.method().as_str(), path) {
        ("POST", "/openai/v1/responses") => (
            format!("openai/{}/v1/responses", state.instance),
            Some(Protocol::Responses),
        ),
        ("POST", "/openai/v1/chat/completions") => (
            format!("openai/{}/v1/chat/completions", state.instance),
            Some(Protocol::Chat),
        ),
        ("POST", "/anthropic/v1/messages") => (
            format!("anthropic/{}/v1/messages", state.instance),
            Some(Protocol::Anthropic),
        ),
        ("POST", "/anthropic/v1/messages/count_tokens") => (
            format!("anthropic/{}/v1/messages/count_tokens", state.instance),
            Some(Protocol::Anthropic),
        ),
        ("GET", "/openai/v1/models") => (format!("openai/{}/v1/models", state.instance), None),
        _ => return error(404, "adapter route is unsupported"),
    };
    if request
        .headers()
        .get("content-encoding")
        .is_some_and(|v| v != "identity")
    {
        return error(415, "compressed adapter uploads are unsupported");
    }
    let permit = match state.slots.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return error(429, "private adapter is busy; retry shortly"),
    };
    let (parts, mut body) = request.into_parts();
    let payload = if let Some(protocol) = protocol {
        let (sender, receiver) = mpsc::sync_channel(2);
        let archive = state.archive.clone();
        let parsing = tokio::task::spawn_blocking(move || {
            // The permit remains held until blocking parsing exits, including cancellation.
            let _permit = permit;
            let mut save =
                |id: &str, part: &Value, protocol| archive.lock().ok()?.save(id, part, protocol);
            image_history::project_reader(
                std::io::BufReader::new(ChunkReader {
                    receiver,
                    current: std::io::Cursor::new(Bytes::new()),
                }),
                protocol,
                16,
                Some(&mut save),
            )
        });
        let feeding = async {
            while let Some(frame) = body.frame().await {
                let frame = frame.map_err(|_| ())?;
                if let Ok(data) = frame.into_data() {
                    // Backpressure without blocking the asynchronous transport workers.
                    let sender = sender.clone();
                    tokio::task::spawn_blocking(move || sender.send(data))
                        .await
                        .map_err(|_| ())?
                        .map_err(|_| ())?;
                }
            }
            drop(sender);
            Ok::<_, ()>(())
        };
        let fed = tokio::time::timeout(Duration::from_secs(120), feeding).await;
        if fed.is_err() {
            return error(408, "private adapter upload timed out");
        }
        match tokio::time::timeout(Duration::from_secs(120), parsing).await {
            Ok(Ok(Ok((value, _)))) if matches!(fed, Ok(Ok(()))) => match serde_json::to_vec(&value)
            {
                Ok(bytes) => Some(bytes),
                Err(_) => return error(400, "request cannot be encoded"),
            },
            Ok(Ok(Err(message))) => {
                return error(
                    if message.contains("byte budget") || message.contains("32 MiB") {
                        413
                    } else {
                        400
                    },
                    message,
                );
            }
            _ => {
                return error(
                    413,
                    "history cannot fit the adapter budgets; compact text or split the new image batch",
                );
            }
        }
    } else {
        drop(permit);
        None
    };
    let Ok(mut url) = state.base.join(&route) else {
        return error(502, "invalid pinned upstream route");
    };
    if let Some(query) = parts.uri.query() {
        url.set_query(Some(query));
    }
    let mut upstream = state
        .http
        .request(parts.method, url)
        .bearer_auth(&state.remote_token);
    for name in ["anthropic-version", "anthropic-beta", "accept"] {
        if let Some(value) = parts.headers.get(name) {
            upstream = upstream.header(name, value);
        }
    }
    if let Some(payload) = payload {
        upstream = upstream
            .header("content-type", "application/json")
            .body(payload);
    }
    let response = match upstream.send().await {
        Ok(r) => r,
        Err(_) => return error(502, "pinned Spark inference transport is unavailable"),
    };
    let mut builder = Response::builder().status(response.status());
    for name in ["content-type", "retry-after"] {
        if let Some(value) = response.headers().get(name) {
            builder = builder.header(name, value);
        }
    }
    let stream = futures_util::stream::try_unfold(response, |mut response| async move {
        match response.chunk().await {
            Ok(Some(chunk)) => Ok(Some((Frame::data(chunk), response))),
            Ok(None) => Ok(None),
            Err(_) => Err(io::Error::other("inference response stream disconnected")),
        }
    });
    builder
        .body(StreamBody::new(stream).boxed_unsync())
        .expect("upstream status and headers")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct HistoryReader {
        next: usize,
        current: io::Cursor<Vec<u8>>,
        image: String,
        read: Arc<AtomicUsize>,
    }
    impl Read for HistoryReader {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            loop {
                let n = self.current.read(out)?;
                if n > 0 {
                    self.read.fetch_add(n, Ordering::Relaxed);
                    return Ok(n);
                }
                let bytes = match self.next {
                    0 => br#"{"input":["#.to_vec(),
                    1..=200 => {
                        let i = self.next - 1;
                        format!("{}{}", if i == 0 { "" } else { "," },
                            json!({"type":"message","role":"user","content":[{"type":"input_text","text":format!("frame {i}")},{"type":"input_image","image_url":self.image}]})).into_bytes()
                    }
                    201 => b"],\"stream\":true}".to_vec(),
                    _ => return Ok(0),
                };
                // Each inspected frame is followed by an assistant observation.
                let bytes = if (1..200).contains(&self.next) {
                    let mut bytes = bytes;
                    bytes.extend_from_slice(format!(",{}", json!({"type":"message","role":"assistant","content":"Button remains visible."})).as_bytes());
                    bytes
                } else {
                    bytes
                };
                self.next += 1;
                self.current = io::Cursor::new(bytes);
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn streamed_history_exceeds_gateway_upload_limit_but_forwarded_body_stays_small() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let observed = sent.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let observed = observed.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request: Request<Incoming>| {
                        let observed = observed.clone();
                        async move {
                            assert_eq!(request.uri().path(), "/openai/fixture/v1/responses");
                            assert_eq!(request.headers()["authorization"], "Bearer REMOTE_FIXTURE");
                            let bytes = request.into_body().collect().await.unwrap().to_bytes();
                            assert!(bytes.len() < image_history::REQUEST_BYTES);
                            observed
                                .lock()
                                .unwrap()
                                .push(serde_json::from_slice::<Value>(&bytes).unwrap());
                            Ok::<_, std::convert::Infallible>(Response::new(Full::new(
                                Bytes::from_static(b"data: {\"type\":\"response.completed\"}\n\n"),
                            )))
                        }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("archive");
        super::super::economics::private_dir(&archive).unwrap();
        let adapter = Adapter::start_with(
            reqwest::Client::new(),
            format!("http://{address}").parse().unwrap(),
            "fixture",
            "REMOTE_FIXTURE",
            archive.clone(),
        )
        .unwrap();
        let http = reqwest::Client::new();
        let unauthorized = http
            .post(format!("{}/openai/v1/responses", adapter.base_url))
            .body("bad")
            .send()
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), 401);
        let forbidden = http
            .post(format!("{}/api/sparkplane/v1/tokens", adapter.base_url))
            .bearer_auth(adapter.token())
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(forbidden.status(), 404);
        assert!(sent.lock().unwrap().is_empty());
        let malformed = http
            .post(format!("{}/openai/v1/responses", adapter.base_url))
            .bearer_auth(adapter.token())
            .body("{")
            .send()
            .await
            .unwrap();
        assert_eq!(malformed.status(), 400);
        let excessive = json!({"input":[{"type":"message","role":"user","content":(0..17).map(|_| json!({"type":"input_image","image_url":"data:image/png;base64,aW1hZ2U="})).collect::<Vec<_>>()}]});
        let excessive = http
            .post(format!("{}/openai/v1/responses", adapter.base_url))
            .bearer_auth(adapter.token())
            .json(&excessive)
            .send()
            .await
            .unwrap();
        assert_eq!(excessive.status(), 400);
        assert!(excessive.text().await.unwrap().contains("split the batch"));
        assert!(sent.lock().unwrap().is_empty());
        let config: toml::Value = toml::from_str(include_str!(
            "../../configs/sparkplane/engines/vllm-qwen38-mmap.toml"
        ))
        .unwrap();
        let mut original = BASE64
            .decode(
                config["profiles"][0]["vision"]["health_image_base64"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
        original.resize(200_000, 0);
        let image = format!("data:image/png;base64,{}", BASE64.encode(&original));
        let bytes_read = Arc::new(AtomicUsize::new(0));
        let reader = HistoryReader {
            next: 0,
            current: io::Cursor::new(Vec::new()),
            image,
            read: bytes_read.clone(),
        };
        let endpoint = format!("{}/openai/v1/responses", adapter.base_url);
        let token = adapter.token().to_owned();
        let response = tokio::task::spawn_blocking(move || {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap()
                .post(endpoint)
                .bearer_auth(token)
                .body(reqwest::blocking::Body::new(reader))
                .send()
                .unwrap()
                .text()
                .unwrap()
        })
        .await
        .unwrap();
        assert!(response.contains("response.completed"), "{response}");
        assert!(bytes_read.load(Ordering::Relaxed) > image_history::REQUEST_BYTES);
        let value = sent.lock().unwrap().pop().unwrap();
        let mut projected = value.clone();
        let mut images = 0;
        image_history::visit_parts(&mut projected["input"], Protocol::Responses, &mut |_| {
            images += 1
        });
        assert!(images <= 12);
        assert_eq!(value["input"].as_array().unwrap().len(), 399);
        assert_eq!(value["input"][1]["content"], "Button remains visible.");
        let marker = value["input"][0]["content"][1]["text"].as_str().unwrap();
        let files = fs::read_dir(&archive)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
            .collect::<Vec<_>>();
        assert_eq!(files.len(), 1);
        assert!(marker.contains(&files[0].path().to_string_lossy().to_string()));
        assert_eq!(fs::read(files[0].path()).unwrap(), original);
        assert_eq!(fs::metadata(files[0].path()).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(&archive).unwrap().mode() & 0o777, 0o700);
        let local_address = adapter.base_url.strip_prefix("http://").unwrap().to_owned();
        drop(adapter);
        assert!(tokio::net::TcpStream::connect(local_address).await.is_err());
        server.abort();
    }
}
