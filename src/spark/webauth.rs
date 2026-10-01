//! Finite, workstation-local authentication bridge. It never serves API tokens.
use super::client::SparkClient;
use anyhow::{Result, bail, ensure};
use bytes::Bytes;
use clap::{Args, Parser, Subcommand};
use http_body_util::Full;
use hyper::{Request, Response, body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use std::{
    convert::Infallible,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

pub const ADDRESS: &str = "127.0.0.1:9844";

#[derive(Parser)]
#[command(name = "sparkplane")]
pub struct WebCli {
    #[command(subcommand)]
    pub command: WebCommand,
}
#[derive(Subcommand)]
pub enum WebCommand {
    /// Open the panel and authenticate with a temporary local service.
    Webui(WebArgs),
    /// Start the temporary local service for a panel already open in your browser.
    Webauthsvc {
        #[command(subcommand)]
        command: ServiceCommand,
    },
}
#[derive(Subcommand)]
pub enum ServiceCommand {
    Start(WebArgs),
}
#[derive(Args)]
pub struct WebArgs {
    #[arg(long)]
    pub host: Option<String>,
    #[arg(long, env = "SPARKPLANE_CONFIG_DIR")]
    pub config_dir: Option<PathBuf>,
    #[arg(long, env = "SPARKPLANE_JSON")]
    pub json: bool,
}
struct Helper {
    client: Arc<SparkClient>,
    origin: String,
    intent: Mutex<Option<String>>,
    flow: Mutex<Option<String>>,
}
fn nonce() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
fn cookie<'a>(request: &'a Request<Incoming>, name: &str) -> Option<&'a str> {
    request
        .headers()
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|s| s.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}
fn response(status: u16, body: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("cache-control", "no-store")
        .header("referrer-policy", "no-referrer")
        .header("x-content-type-options", "nosniff")
        .header(
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'",
        )
        .body(Full::new(Bytes::copy_from_slice(body.as_bytes())))
        .expect("fixed response")
}
fn redirect(url: &reqwest::Url, cookie: Option<String>) -> Response<Full<Bytes>> {
    let mut response = response(303, "");
    response
        .headers_mut()
        .insert("location", url.as_str().parse().expect("validated URL"));
    if let Some(cookie) = cookie {
        response
            .headers_mut()
            .insert("set-cookie", cookie.parse().expect("generated cookie"));
    }
    response
}
async fn handle(
    helper: Arc<Helper>,
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    if request.headers().get("host").and_then(|v| v.to_str().ok()) != Some(ADDRESS)
        || request.method() != hyper::Method::GET
    {
        return Ok(response(403, "Local authentication request rejected"));
    }
    let allowed_origin = request
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok());
    if allowed_origin.is_some_and(|origin| origin != helper.origin) {
        return Ok(response(403, "Origin rejected"));
    }
    match request.uri().path() {
        "/health" => {
            let mut response = response(
                200,
                &serde_json::json!({"protocol":"sparkplane.webauth/v1","origin":helper.origin})
                    .to_string(),
            );
            response
                .headers_mut()
                .insert("content-type", "application/json".parse().unwrap());
            if allowed_origin == Some(helper.origin.as_str()) {
                response.headers_mut().insert(
                    "access-control-allow-origin",
                    helper.origin.parse().unwrap(),
                );
                response
                    .headers_mut()
                    .insert("vary", "Origin".parse().unwrap());
            }
            Ok(response)
        }
        "/auth/start" => {
            let intent = nonce();
            *helper.intent.lock().unwrap() = Some(intent.clone());
            let mut url = helper
                .client
                .base_url()
                .join("panel/auth/begin")
                .expect("fixed route");
            url.query_pairs_mut().append_pair("intent", &intent);
            Ok(redirect(
                &url,
                Some(format!(
                    "sparkplane_local_intent={intent}; Path=/auth; HttpOnly; SameSite=Lax; Max-Age=600"
                )),
            ))
        }
        "/auth/approve" => {
            let url = reqwest::Url::parse(&format!("http://{ADDRESS}{}", request.uri()))
                .expect("HTTP request URI");
            let pairs = url.query_pairs().collect::<Vec<_>>();
            let flow = pairs
                .iter()
                .find(|(k, _)| k == "flow")
                .map(|(_, v)| v.to_string());
            let intent = pairs
                .iter()
                .find(|(k, _)| k == "intent")
                .map(|(_, v)| v.to_string());
            let valid = pairs.len() == 2
                && intent.as_deref().is_some_and(|value| {
                    cookie(&request, "sparkplane_local_intent") == Some(value)
                        && helper.intent.lock().unwrap().as_deref() == Some(value)
                })
                && flow.as_ref().is_some_and(|value| {
                    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
                });
            if !valid {
                return Ok(response(
                    403,
                    "Login intent is missing, expired or does not match this browser",
                ));
            }
            let flow = flow.unwrap();
            let intent = intent.unwrap();
            let client = Arc::clone(&helper.client);
            let flow_copy = flow.clone();
            let result =
                tokio::task::spawn_blocking(move || client.approve_web_login(&flow_copy, &intent))
                    .await;
            let Ok(Ok(data)) = result else {
                return Ok(response(
                    502,
                    "Spark rejected authentication; return to the panel and try again",
                ));
            };
            let Some(code) = data["code"]
                .as_str()
                .filter(|code| code.len() == 64 && code.bytes().all(|b| b.is_ascii_hexdigit()))
            else {
                return Ok(response(502, "Invalid authentication response"));
            };
            *helper.intent.lock().unwrap() = None;
            *helper.flow.lock().unwrap() = Some(flow.clone());
            let mut url = helper
                .client
                .base_url()
                .join("panel/auth/finish")
                .expect("fixed route");
            url.query_pairs_mut()
                .append_pair("flow", &flow)
                .append_pair("code", code);
            Ok(redirect(
                &url,
                Some(
                    "sparkplane_local_intent=; Path=/auth; HttpOnly; SameSite=Lax; Max-Age=0"
                        .into(),
                ),
            ))
        }
        _ => Ok(response(404, "Not found")),
    }
}

pub fn dispatch(cli: WebCli) -> Result<()> {
    let (args, open) = match cli.command {
        WebCommand::Webui(args) => (args, true),
        WebCommand::Webauthsvc {
            command: ServiceCommand::Start(args),
        } => (args, false),
    };
    let config = args.config_dir.unwrap_or_else(|| {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join("sparkplane")
    });
    let hosts = SparkClient::configured_hosts(&config)?;
    let host = match args.host {
        Some(host) => host,
        None if hosts.len() == 1 => hosts[0].clone(),
        _ => bail!("select a configured Spark using --host ALIAS"),
    };
    let client = SparkClient::load(&config, &host)?;
    ensure!(
        client.base_url().scheme() == "https",
        "web authentication requires HTTPS"
    );
    client.get_json::<serde_json::Value>("api/sparkplane/v1/web-session")?;
    let mut panel = client.base_url().join("panel/")?;
    if open {
        panel.set_fragment(Some("local-auth"));
    }
    let helper = Arc::new(Helper {
        origin: client.base_url().origin().ascii_serialization(),
        client: Arc::new(client),
        intent: Mutex::new(None),
        flow: Mutex::new(None),
    });
    let listener = std::net::TcpListener::bind(ADDRESS).map_err(|_| {
        anyhow::anyhow!("{ADDRESS} is already occupied; finish or stop the existing local service")
    })?;
    listener.set_nonblocking(true)?;
    eprintln!(
        "Local authentication service ready. Open {panel} and click Authenticate. Expires in ten minutes; Ctrl-C stops it."
    );
    if open {
        match std::process::Command::new("xdg-open")
            .arg(panel.as_str())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(_) => eprintln!("Could not open your browser; open {panel} manually."),
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let complete = runtime.block_on(serve(listener, helper))?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({"schema":"sparkplane.webauth/v1","host":host,"authenticated":complete,"service_stopped":true})
        );
    }
    if !complete {
        bail!("local authentication service stopped before authentication completed");
    }
    eprintln!("Authenticated. Local authentication service stopped.");
    Ok(())
}
async fn serve(listener: std::net::TcpListener, helper: Arc<Helper>) -> Result<bool> {
    let listener = tokio::net::TcpListener::from_std(listener)?;
    let slots = Arc::new(Semaphore::new(16));
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let expiry = tokio::time::sleep(Duration::from_secs(600));
    let mut connections = tokio::task::JoinSet::new();
    tokio::pin!(expiry);
    let completed = loop {
        tokio::select! {
            _ = &mut expiry => break false,
            _ = tokio::signal::ctrl_c() => break false,
            _ = connections.join_next(), if !connections.is_empty() => {},
            _ = tick.tick() => {
                let flow = helper.flow.lock().unwrap().clone();
                if let Some(flow) = flow {
                    let client = Arc::clone(&helper.client);
                    if tokio::task::spawn_blocking(move || client.web_login_complete(&flow)).await.is_ok_and(|r| r.unwrap_or(false)) { break true; }
                }
            },
            accepted = listener.accept() => {
                let (stream,peer) = accepted?;
                if !peer.ip().is_loopback() { continue; }
                let Ok(permit) = slots.clone().try_acquire_owned() else { continue; };
                let helper = helper.clone();
                connections.spawn(async move {
                    let _permit = permit;
                    let connection = http1::Builder::new().max_buf_size(16*1024).serve_connection(TokioIo::new(stream),service_fn(move |req|handle(helper.clone(),req)));
                    let _ = tokio::time::timeout(Duration::from_secs(10),connection).await;
                });
            }
        }
    };
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    Ok(completed)
}

#[cfg(all(test, feature = "appliance"))]
pub(crate) async fn test_service(config: PathBuf, listener: std::net::TcpListener) -> Result<bool> {
    let client =
        tokio::task::spawn_blocking(move || SparkClient::load(&config, "fixture")).await??;
    let helper = Arc::new(Helper {
        origin: client.base_url().origin().ascii_serialization(),
        client: Arc::new(client),
        intent: Mutex::new(None),
        flow: Mutex::new(None),
    });
    serve(listener, helper).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostless_commands_parse_without_changing_host_first_cli() {
        assert!(matches!(
            WebCli::try_parse_from(["sparkplane", "webui"])
                .unwrap()
                .command,
            WebCommand::Webui(_)
        ));
        assert!(matches!(
            WebCli::try_parse_from(["sparkplane", "webauthsvc", "start", "--host", "spark"])
                .unwrap()
                .command,
            WebCommand::Webauthsvc { .. }
        ));
        assert!(
            WebCli::try_parse_from(["sparkplane", "webauthsvc", "start", "--token", "secret"])
                .is_err()
        );
    }
}
