//! Browser sessions and panel APIs. The original bearer never enters the browser.

use super::*;
use axum::response::{Html, Redirect};
use serde::Serialize;
use std::{collections::HashMap, sync::Mutex, time::Instant};

include!(concat!(env!("OUT_DIR"), "/panel_assets.rs"));

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PanelConfig {
    pub enabled: bool,
    pub origin: Option<String>,
}
impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            origin: None,
        }
    }
}

#[derive(Default)]
pub(super) struct WebState {
    pub config: PanelConfig,
    flows: Mutex<HashMap<String, LoginFlow>>,
    sessions: Mutex<HashMap<String, WebSession>>,
    health: Mutex<Option<serde_json::Value>>,
}
struct LoginFlow {
    intent: String,
    browser_hash: String,
    origin: String,
    created: Instant,
    approval: Option<(String, String, Instant)>,
    consumed: bool,
}
struct WebSession {
    token_id: String,
    origin: String,
    csrf: String,
    created: Instant,
    touched: Instant,
}
pub(super) fn secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn valid_secret(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}
// HTTP/2 sends :authority rather than Host. Preserve one validated identity for
// both transports before origin checks and header extractors run.
pub(super) async fn normalize_authority(mut request: Request, next: Next) -> Response {
    if let Some(authority) = request.uri().authority() {
        let Ok(host) = HeaderValue::from_str(authority.as_str()) else {
            return denied();
        };
        if request
            .headers()
            .get(header::HOST)
            .is_some_and(|existing| !existing.as_bytes().eq_ignore_ascii_case(host.as_bytes()))
        {
            return denied();
        }
        request.headers_mut().insert(header::HOST, host);
    }
    next.run(request).await
}
fn origin(state: &AgentState, headers: &HeaderMap) -> Option<String> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let url = reqwest::Url::parse(&format!("https://{host}")).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let hostname = url.host_str()?.trim_matches(['[', ']']);
    if !state
        .certificate
        .dns_sans
        .iter()
        .chain(&state.certificate.ip_sans)
        .any(|san| san.eq_ignore_ascii_case(hostname))
    {
        return None;
    }
    let origin = url.origin().ascii_serialization();
    if state
        .web
        .config
        .origin
        .as_ref()
        .is_some_and(|configured| configured != &origin)
    {
        return None;
    }
    Some(origin)
}
pub(super) fn validate_config(state: &AgentState) -> anyhow::Result<()> {
    if let Some(configured) = &state.web.config.origin {
        let url = reqwest::Url::parse(configured)?;
        anyhow::ensure!(
            url.scheme() == "https"
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none()
                && url.origin().ascii_serialization() == *configured,
            "webui.origin must be an exact HTTPS origin without a path"
        );
        let mut headers = HeaderMap::new();
        let host = configured
            .strip_prefix("https://")
            .expect("validated scheme");
        headers.insert(header::HOST, HeaderValue::from_str(host)?);
        anyhow::ensure!(
            origin(state, &headers).is_some(),
            "webui.origin must match an installed certificate identity"
        );
    }
    Ok(())
}
fn secure(mut response: Response) -> Response {
    response
        .headers_mut()
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    for (name, value) in [
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' http://127.0.0.1:9844; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    ] {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    response
}
fn denied() -> Response {
    secure(auth_failed("/panel/"))
}
fn set_cookie(response: &mut Response, value: String) {
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&value).expect("generated cookie"),
    );
}

pub(super) fn public_routes(state: AgentState) -> Router<AgentState> {
    Router::new()
        .route("/panel", get(|| async { Redirect::to("/panel/") }))
        .route("/panel/", get(asset))
        .route("/panel/auth/begin", get(begin))
        .route("/panel/auth/finish", get(finish))
        .route(
            "/panel/auth/complete.js",
            get(|| async {
                secure(
                    (
                        [(header::CONTENT_TYPE, "text/javascript")],
                        "window.location.replace('/panel/');",
                    )
                        .into_response(),
                )
            }),
        )
        .route("/panel/{*path}", get(asset))
        .route_layer(middleware::from_fn_with_state(state, public_guard))
}
async fn public_guard(State(state): State<AgentState>, request: Request, next: Next) -> Response {
    if !state.web.config.enabled
        || !peer_allowed(
            &state,
            request.extensions().get::<ConnectInfo<SocketAddr>>(),
        )
        || origin(&state, request.headers()).is_none()
    {
        return denied();
    }
    secure(next.run(request).await)
}
pub(super) fn api_routes() -> Router<AgentState> {
    Router::new()
        .route(
            &format!("{API_BASE}/web-auth/approve"),
            axum::routing::post(approve),
        )
        .route(
            &format!("{API_BASE}/web-auth/flows/{{id}}"),
            get(flow_status),
        )
        .route(&format!("{API_BASE}/web-session"), get(me).delete(logout))
        .route(&format!("{API_BASE}/analytics"), get(analytics_summary))
        .route(
            &format!("{API_BASE}/analytics/requests"),
            get(analytics_requests),
        )
        .route(
            &format!("{API_BASE}/analytics/sessions"),
            get(analytics_sessions),
        )
        .route(&format!("{API_BASE}/audit"), get(audit))
        .route(&format!("{API_BASE}/health"), get(health))
        .route(&format!("{API_BASE}/health/history"), get(health_history))
        .route(
            &format!("{API_BASE}/launch-sessions/{{id}}/compression"),
            axum::routing::post(session_compression),
        )
        .route(
            &format!("{API_BASE}/instances/{{id}}/recover"),
            axum::routing::post(recover),
        )
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct RecoverRequest {
    #[serde(default)]
    dry_run: bool,
}
async fn recover(
    State(state): State<AgentState>,
    Extension(auth): Extension<AuthenticatedToken>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(body): Json<RecoverRequest>,
) -> Response {
    let Some(db) = state.database.clone() else {
        return database_unavailable();
    };
    let instance = match db.instance(&id).await {
        Ok(i) => i,
        Err(error) => return state_problem(error),
    };
    if let Err(detail) = recovery_settings(&state, &instance).await {
        return problem(
            StatusCode::CONFLICT,
            "spark.instance.recovery-settings",
            &detail,
        );
    }
    if body.dry_run {
        return Json(serde_json::json!({"instance":instance,"actions":["stop","serve"],"preserves":"qualified engine settings","interrupts_active_requests":true})).into_response();
    }
    if let Err(response) = require_executor_for_mutation(&state).await {
        return response;
    }
    let key = match required_idempotency(&headers) {
        Ok(key) => key,
        Err(()) => return missing_idempotency(),
    };
    let hash =
        super::super::wire::canonical_request_sha256(&serde_json::json!({"instance":instance.id}))
            .expect("finite recovery intent");
    let accepted = match db
        .accept_operation(
            &auth.id,
            "instance.recover",
            &key,
            &hash,
            Some(instance.id.clone()),
        )
        .await
    {
        Ok(op) => op,
        Err(error) => return state_problem(error),
    };
    if !accepted.reused {
        let operation = accepted.operation.id.clone();
        tokio::spawn(async move {
            let result = run_recovery(state.clone(), auth, instance, &operation).await;
            let state_value = if result.is_ok() {
                super::super::wire::OperationState::Succeeded
            } else {
                super::super::wire::OperationState::Failed
            };
            let problem = result.err().map(|detail| ProblemDocument {
                schema: PROBLEM_SCHEMA.into(),
                r#type: "about:blank".into(),
                code: "spark.instance.recovery-failed".into(),
                status: 409,
                detail,
                remediation: vec![
                    "Inspect the child operations and instance state before retrying.".into(),
                ],
                operation_id: Some(operation.clone()),
            });
            let _ = db
                .transition(
                    &operation,
                    state_value,
                    OperationProgress {
                        stage: "finished".into(),
                        current: None,
                        total: None,
                        unit: None,
                        message: "Managed recovery finished".into(),
                    },
                    None,
                    problem,
                )
                .await;
        });
    }
    accepted_response(accepted.operation)
}
async fn run_recovery(
    state: AgentState,
    auth: AuthenticatedToken,
    instance: InstanceDocument,
    parent: &str,
) -> Result<(), String> {
    let db = state.database.as_ref().ok_or("database unavailable")?;
    db.transition(
        parent,
        super::super::wire::OperationState::Running,
        lifecycle_progress("validating", "Validating qualified recovery settings"),
        None,
        None,
    )
    .await
    .map_err(|e| e.to_string())?;
    for phase in ["stop", "serve"] {
        recovery_settings(&state, &instance).await?;
        let current = db.operation(parent).await.map_err(|e| e.to_string())?;
        if current.state.is_terminal() {
            return Err("recovery no longer active".into());
        }
        let _ = db
            .transition(
                parent,
                super::super::wire::OperationState::Running,
                OperationProgress {
                    stage: phase.into(),
                    current: None,
                    total: None,
                    unit: None,
                    message: format!("Managed recovery: {phase}"),
                },
                None,
                None,
            )
            .await;
        let mut headers = HeaderMap::new();
        headers.insert(
            "idempotency-key",
            HeaderValue::from_str(&format!("{parent}-{phase}")).expect("bounded key"),
        );
        let response = if phase == "stop" {
            stop_instance(
                State(state.clone()),
                AxumPath(instance.id.clone()),
                headers,
                Extension(auth.clone()),
                Json(StopRequest {
                    timeout_seconds: 30,
                    dry_run: false,
                }),
            )
            .await
        } else {
            serve_instance(
                State(state.clone()),
                headers,
                Extension(auth.clone()),
                Json(ServeRequest {
                    model: instance.model.clone(),
                    name: Some(instance.name.clone()),
                    dry_run: false,
                }),
            )
            .await
        };
        if !response.status().is_success() {
            return Err(format!(
                "{phase} was rejected; inspect instance health and admission"
            ));
        }
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .map_err(|_| "child response unavailable")?;
        let child: OperationDocument =
            serde_json::from_slice(&bytes).map_err(|_| "child operation unavailable")?;
        let deadline = Instant::now()
            + Duration::from_secs(super::super::MAX_ENGINE_STARTUP_DEADLINE_SECONDS + 60);
        loop {
            let current = db.operation(&child.id).await.map_err(|e| e.to_string())?;
            if current.state.is_terminal() {
                if current.state != super::super::wire::OperationState::Succeeded {
                    return Err(format!("{phase} operation did not succeed"));
                }
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{phase} operation remains pending; inspect its progress"
                ));
            }
            if db
                .operation(parent)
                .await
                .map_err(|e| e.to_string())?
                .state
                .is_terminal()
            {
                return Err(
                    "recovery cancelled; child operation remains visible in Operations".into(),
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    Ok(())
}
async fn recovery_settings(state: &AgentState, instance: &InstanceDocument) -> Result<(), String> {
    let db = state.database.as_ref().ok_or("database unavailable")?;
    let model = db
        .model(&instance.model_id)
        .await
        .map_err(|e| e.to_string())?;
    let artifacts = model
        .artifacts
        .as_ref()
        .ok_or("verified artifact traits unavailable")?;
    let policy = state
        .engine_catalog
        .as_ref()
        .ok_or("engine catalog unavailable")?
        .select(artifacts)?;
    let profile = policy.profile_for(None, artifacts)?;
    if policy.fingerprint() != instance.engine_fingerprint
        || policy.config().id != instance.engine_id
        || model.canonical != instance.model
        || model.commit != instance.model_commit
        || artifacts != &instance.artifacts
        || instance.objective != "inference"
        || profile.context_window != instance.context_window
        || profile.sampling.default_reasoning_effort != instance.default_reasoning_effort
        || policy.resources_for(None, artifacts)? != instance.resources
    {
        return Err("Current catalog differs from this instance's qualified settings; use the signed capability activation workflow.".into());
    }
    Ok(())
}

async fn panel_query(
    state: AgentState,
    query: crate::spark::analytics::AnalyticsQuery,
    kind: crate::spark::analytics::QueryKind,
) -> Response {
    let Some(db) = &state.database else {
        return database_unavailable();
    };
    match db.panel_query(kind, query).await {
        Ok(value) => secure(Json(value).into_response()),
        Err(error) => state_problem(error),
    }
}
async fn analytics_summary(
    State(state): State<AgentState>,
    Query(query): Query<crate::spark::analytics::AnalyticsQuery>,
) -> Response {
    panel_query(state, query, crate::spark::analytics::QueryKind::Summary).await
}
async fn analytics_requests(
    State(state): State<AgentState>,
    Query(query): Query<crate::spark::analytics::AnalyticsQuery>,
) -> Response {
    panel_query(state, query, crate::spark::analytics::QueryKind::Requests).await
}
async fn analytics_sessions(
    State(state): State<AgentState>,
    Query(query): Query<crate::spark::analytics::AnalyticsQuery>,
) -> Response {
    panel_query(state, query, crate::spark::analytics::QueryKind::Sessions).await
}
async fn audit(
    State(state): State<AgentState>,
    Query(query): Query<crate::spark::analytics::AnalyticsQuery>,
) -> Response {
    panel_query(state, query, crate::spark::analytics::QueryKind::Audit).await
}
async fn health_history(
    State(state): State<AgentState>,
    Query(query): Query<crate::spark::analytics::AnalyticsQuery>,
) -> Response {
    panel_query(state, query, crate::spark::analytics::QueryKind::Health).await
}
async fn session_compression(
    State(state): State<AgentState>,
    Extension(auth): Extension<AuthenticatedToken>,
    AxumPath(id): AxumPath<String>,
    Json(report): Json<crate::spark::economics::Compression>,
) -> Response {
    let Some(db) = &state.database else {
        return database_unavailable();
    };
    match db.session_compression(&auth.id, &id, report).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => state_problem(error),
    }
}
async fn health(State(state): State<AgentState>) -> Response {
    let sample = state
        .web
        .health
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    match sample {
        Some(sample) => secure(Json(sample).into_response()),
        None => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "spark.health.pending",
            "health sample is not available yet",
        ),
    }
}
pub(super) async fn sample_health(state: AgentState) {
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    let mut cycle = 0u64;
    let mut previous_cpu: Option<(u64, u64)> = None;
    loop {
        tick.tick().await;
        let telemetry = if let Some(executor) = &state.executor {
            tokio::time::timeout(Duration::from_secs(3), executor.panel_telemetry())
                .await
                .ok()
                .and_then(Result::ok)
        } else {
            None
        };
        let database = if let Some(db) = &state.database {
            db.health().await.ok()
        } else {
            None
        };
        let cpu = telemetry
            .as_ref()
            .and_then(|t| t.cpu_total_ticks.zip(t.cpu_idle_ticks));
        let cpu_percent = cpu
            .zip(previous_cpu)
            .and_then(|((total, idle), (old_total, old_idle))| {
                total.checked_sub(old_total).zip(idle.checked_sub(old_idle))
            })
            .filter(|(total, idle)| *total > 0 && idle <= total)
            .map(|(total, idle)| 100.0 * (total - idle) as f64 / total as f64);
        previous_cpu = cpu;
        let sample = serde_json::json!({"schema":"sparkplane.health/v1","observed_at_unix_ms":unix_millis(),"agent_version":env!("CARGO_PKG_VERSION"),"cpu_percent":cpu_percent,"executor":telemetry,"database":database});
        *state
            .web
            .health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sample.clone());
        if let Some(db) = &state.database {
            let _ = db.panel_health(sample).await;
            if cycle.is_multiple_of(12) {
                let _ = db.panel_cleanup().await;
            }
        }
        cycle = cycle.wrapping_add(1);
    }
}
fn peer_allowed(state: &AgentState, peer: Option<&ConnectInfo<SocketAddr>>) -> bool {
    peer.is_none_or(|peer| {
        state
            .allowed_clients
            .iter()
            .any(|cidr| cidr.contains(peer.0.ip()))
    })
}
async fn asset(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
) -> Response {
    let peer = peer.map(|Extension(peer)| peer);
    if !state.web.config.enabled
        || !peer_allowed(&state, peer.as_ref())
        || origin(&state, &headers).is_none()
    {
        return denied();
    }
    let path = uri.path().strip_prefix("/panel/").unwrap_or("");
    let file = if path.is_empty()
        || [
            "overview",
            "analytics",
            "models",
            "instances",
            "operations",
            "health",
            "admin",
        ]
        .contains(&path)
    {
        "index.html"
    } else {
        path
    };
    match ASSETS.iter().find(|(name, _, _)| *name == file) {
        Some((_, bytes, mime)) => {
            let mut response = secure(([(header::CONTENT_TYPE, *mime)], *bytes).into_response());
            if file.starts_with("assets/") {
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=31536000, immutable"),
                );
            }
            response
        }
        None => not_found().await,
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BeginQuery {
    intent: String,
}
async fn begin(
    State(state): State<AgentState>,
    headers: HeaderMap,
    Query(query): Query<BeginQuery>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
) -> Response {
    let peer = peer.map(|Extension(peer)| peer);
    if !state.web.config.enabled
        || !valid_secret(&query.intent)
        || !peer_allowed(&state, peer.as_ref())
    {
        return denied();
    }
    let Some(origin) = origin(&state, &headers) else {
        return denied();
    };
    let mut flows = state
        .web
        .flows
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    flows.retain(|_, flow| flow.created.elapsed() < Duration::from_secs(600));
    if flows.len() >= 128 {
        return problem(
            StatusCode::TOO_MANY_REQUESTS,
            "spark.web.busy",
            "too many pending logins",
        );
    }
    let id = secret();
    let browser = secret();
    flows.insert(
        id.clone(),
        LoginFlow {
            intent: query.intent.clone(),
            browser_hash: hash(&browser),
            origin,
            created: Instant::now(),
            approval: None,
            consumed: false,
        },
    );
    let mut response = Redirect::to(&format!(
        "http://127.0.0.1:9844/auth/approve?flow={id}&intent={}",
        query.intent
    ))
    .into_response();
    set_cookie(
        &mut response,
        format!(
            "sparkplane_web_pending={browser}; Path=/panel/auth; HttpOnly; Secure; SameSite=Lax; Max-Age=600"
        ),
    );
    secure(response)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRequest {
    flow: String,
    intent: String,
}
async fn approve(
    State(state): State<AgentState>,
    Extension(auth): Extension<AuthenticatedToken>,
    Json(body): Json<ApprovalRequest>,
) -> Response {
    if !state.web.config.enabled {
        return denied();
    }
    let mut flows = state
        .web
        .flows
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(flow) = flows.get_mut(&body.flow) else {
        return denied();
    };
    if flow.created.elapsed() >= Duration::from_secs(600)
        || flow.consumed
        || flow.approval.is_some()
        || !token_matches(&flow.intent, &body.intent)
    {
        return denied();
    }
    let code = secret();
    flow.approval = Some((auth.id, hash(&code), Instant::now()));
    secure(Json(serde_json::json!({"code":code})).into_response())
}
async fn flow_status(
    State(state): State<AgentState>,
    Extension(auth): Extension<AuthenticatedToken>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let flows = state
        .web
        .flows
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match flows.get(&id) {
        Some(flow)
            if flow.created.elapsed() < Duration::from_secs(600)
                && flow
                    .approval
                    .as_ref()
                    .is_some_and(|(token, _, _)| token == &auth.id) =>
        {
            secure(Json(serde_json::json!({"complete":flow.consumed})).into_response())
        }
        _ => denied(),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishQuery {
    flow: String,
    code: String,
}
async fn finish(
    State(state): State<AgentState>,
    headers: HeaderMap,
    Query(query): Query<FinishQuery>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
) -> Response {
    let peer = peer.map(|Extension(peer)| peer);
    if !state.web.config.enabled || !peer_allowed(&state, peer.as_ref()) {
        return denied();
    }
    let Some(origin) = origin(&state, &headers) else {
        return denied();
    };
    let Some(browser) = cookie(&headers, "sparkplane_web_pending") else {
        return denied();
    };
    let mut flows = state
        .web
        .flows
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(flow) = flows.get_mut(&query.flow) else {
        return denied();
    };
    let Some((token_id, code_hash, approved)) = flow.approval.as_ref() else {
        return denied();
    };
    if flow.consumed
        || flow.origin != origin
        || approved.elapsed() >= Duration::from_secs(60)
        || flow.created.elapsed() >= Duration::from_secs(600)
        || !token_matches(&flow.browser_hash, &hash(browser))
        || !token_matches(code_hash, &hash(&query.code))
        || token_identity(&state, token_id, peer.as_ref()).is_none()
    {
        return denied();
    }
    let mut sessions = state
        .web
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    sessions.retain(|_, session| {
        session.created.elapsed() < Duration::from_secs(8 * 3600)
            && session.touched.elapsed() < Duration::from_secs(1800)
    });
    if sessions.len() >= 1024 {
        return denied();
    }
    let session = secret();
    sessions.insert(
        hash(&session),
        WebSession {
            token_id: token_id.clone(),
            origin,
            csrf: secret(),
            created: Instant::now(),
            touched: Instant::now(),
        },
    );
    flow.consumed = true;
    let mut response = Html("<!doctype html><html lang=\"en\"><head><title>Authenticated · Sparkplane</title><script src=\"/panel/auth/complete.js\" defer></script></head><body><a href=\"/panel/\">Open Sparkplane</a></body></html>").into_response();
    set_cookie(
        &mut response,
        format!(
            "sparkplane_web={session}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=28800"
        ),
    );
    set_cookie(
        &mut response,
        "sparkplane_web_pending=; Path=/panel/auth; HttpOnly; Secure; SameSite=Lax; Max-Age=0"
            .into(),
    );
    secure(response)
}
fn token_identity(
    state: &AgentState,
    id: &str,
    peer: Option<&ConnectInfo<SocketAddr>>,
) -> Option<AuthenticatedToken> {
    if id == "bootstrap-admin" {
        return Some(AuthenticatedToken::admin(state.inference_slot(id, 64)));
    }
    let snapshot = state.auth.load();
    let token = &snapshot.tokens.get(id)?.token;
    if token.revoked_at.is_some()
        || token.expires_at.as_ref().is_some_and(|expiry| {
            chrono::DateTime::parse_from_rfc3339(expiry)
                .map_or(true, |expiry| expiry <= chrono::Utc::now())
        })
    {
        return None;
    }
    if !token.allowed_cidrs.is_empty()
        && !peer.is_some_and(|peer| {
            token
                .allowed_cidrs
                .iter()
                .filter_map(|c| c.parse::<Cidr>().ok())
                .any(|c| c.contains(peer.0.ip()))
        })
    {
        return None;
    }
    Some(AuthenticatedToken {
        id: id.into(),
        scopes: token.scopes.clone(),
        inference: state.inference_slot(id, token.max_concurrent_inference),
    })
}
pub(super) fn authenticate_cookie(
    state: &AgentState,
    request: &Request,
) -> Option<AuthenticatedToken> {
    if !state.web.config.enabled
        || !request.uri().path().starts_with(API_BASE)
        || request
            .uri()
            .path()
            .starts_with(&format!("{API_BASE}/web-auth"))
        || request.headers().contains_key(header::AUTHORIZATION)
    {
        return None;
    }
    let session = cookie(request.headers(), "sparkplane_web")?;
    let origin = origin(state, request.headers())?;
    if request
        .headers()
        .get("sec-fetch-site")
        .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return None;
    }
    let mut sessions = state
        .web
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let session = sessions.get_mut(&hash(session))?;
    if session.origin != origin
        || session.created.elapsed() >= Duration::from_secs(8 * 3600)
        || session.touched.elapsed() >= Duration::from_secs(1800)
    {
        return None;
    }
    if ![Method::GET, Method::HEAD].contains(request.method())
        && (request
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            != Some(origin.as_str())
            || !request
                .headers()
                .get("x-sparkplane-csrf")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| token_matches(&session.csrf, v)))
    {
        return None;
    }
    if request
        .headers()
        .get(header::ORIGIN)
        .is_some_and(|v| v.to_str().ok() != Some(origin.as_str()))
    {
        return None;
    }
    let auth = token_identity(
        state,
        &session.token_id,
        request.extensions().get::<ConnectInfo<SocketAddr>>(),
    )?;
    session.touched = Instant::now();
    Some(auth)
}
#[derive(Serialize)]
struct SessionDocument {
    token_id: String,
    scopes: Vec<TokenScope>,
    csrf: Option<String>,
}
async fn me(
    State(state): State<AgentState>,
    headers: HeaderMap,
    Extension(auth): Extension<AuthenticatedToken>,
) -> Response {
    let csrf = cookie(&headers, "sparkplane_web").and_then(|value| {
        state
            .web
            .sessions
            .lock()
            .ok()?
            .get(&hash(value))
            .map(|s| s.csrf.clone())
    });
    secure(
        Json(SessionDocument {
            token_id: auth.id,
            scopes: auth.scopes,
            csrf,
        })
        .into_response(),
    )
}
async fn logout(State(state): State<AgentState>, headers: HeaderMap) -> Response {
    if let Some(value) = cookie(&headers, "sparkplane_web") {
        state
            .web
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&hash(value));
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    set_cookie(
        &mut response,
        "sparkplane_web=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0".into(),
    );
    secure(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn session_lifetimes_logout_and_http2_authority_are_enforced() {
        let state = AgentState::new("fixture-secret", vec!["localhost".into()], vec![]);
        let value = secret();
        let csrf = secret();
        state.web.sessions.lock().unwrap().insert(
            hash(&value),
            WebSession {
                token_id: "bootstrap-admin".into(),
                origin: "https://localhost".into(),
                csrf: csrf.clone(),
                created: Instant::now(),
                touched: Instant::now(),
            },
        );
        let app = super::super::router(state.clone());
        let make = || {
            Request::builder()
                .uri("https://localhost/api/sparkplane/v1/web-session")
                .version(axum::http::Version::HTTP_2)
                .header("cookie", format!("sparkplane_web={value}"))
        };
        assert_eq!(
            app.clone()
                .oneshot(make().body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            app.clone()
                .oneshot(
                    make()
                        .header("host", "other-host")
                        .body(Body::empty())
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        for (age, idle) in [(0, 1801), (8 * 3600 + 1, 0)] {
            {
                let mut sessions = state.web.sessions.lock().unwrap();
                let session = sessions.get_mut(&hash(&value)).unwrap();
                session.created = Instant::now() - Duration::from_secs(age);
                session.touched = Instant::now() - Duration::from_secs(idle);
            }
            assert_eq!(
                app.clone()
                    .oneshot(make().body(Body::empty()).unwrap())
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        {
            let mut sessions = state.web.sessions.lock().unwrap();
            let session = sessions.get_mut(&hash(&value)).unwrap();
            session.created = Instant::now();
            session.touched = Instant::now();
        }
        let logout = app
            .clone()
            .oneshot(
                make()
                    .method("DELETE")
                    .header("origin", "https://localhost")
                    .header("x-sparkplane-csrf", csrf)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(logout.status(), StatusCode::NO_CONTENT);
        assert!(
            logout.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
        assert_eq!(
            app.oneshot(make().body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn local_helper_and_browser_sessions_round_trip_over_pinned_https() {
        authentication_fixture(false).await;
    }

    #[tokio::test]
    #[ignore = "requires Node and an installed Playwright Chromium browser"]
    async fn browser_authentication_redirects_over_https() {
        authentication_fixture(true).await;
    }

    async fn authentication_fixture(browser: bool) {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let database = DbActor::open(
            root.path().join("state"),
            root.path().join("backups"),
            64,
            2,
            SecretString::from("fixture-secret"),
        )
        .unwrap();
        let created = database
            .create_token(
                "bootstrap-admin",
                "fixture-reader",
                TokenCreateRequest {
                    name: "panel-reader".into(),
                    scopes: vec![TokenScope::AnalyticsRead],
                    allowed_cidrs: vec!["127.0.0.0/8".into()],
                    expires_at: None,
                    max_concurrent_inference: 1,
                },
            )
            .await
            .unwrap();
        let token = created.bearer_token.unwrap();
        let state = AgentState::new("fixture-secret", vec!["localhost".into()], vec![])
            .with_database(database.clone())
            .await
            .unwrap();
        let server_state = state.clone();
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let certificate = cert.pem();
        let tls = super::super::tls13_config(
            certificate.as_bytes().to_vec(),
            signing_key.serialize_pem().into_bytes(),
        )
        .await
        .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = axum_server::Handle::new();
        let shutdown = handle.clone();
        let server = tokio::spawn(async move {
            axum_server::from_tcp_rustls(listener, tls)
                .unwrap()
                .handle(shutdown)
                .serve(
                    super::super::router(server_state)
                        .into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .unwrap();
        });
        let base = format!("https://localhost:{}", address.port());
        std::fs::create_dir_all(root.path().join("spark")).unwrap();
        std::fs::create_dir_all(root.path().join("credentials")).unwrap();
        std::fs::write(root.path().join("spark/fixture.ca.pem"), &certificate).unwrap();
        std::fs::write(root.path().join("credentials/fixture"), &token).unwrap();
        std::fs::set_permissions(
            root.path().join("credentials/fixture"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        std::fs::write(root.path().join("spark.toml"),format!("[hosts.fixture]\nurl='{base}/'\nca_cert_sha256='sha256:{:x}'\ncredential='fixture'\nrequest_timeout_seconds=5\n",Sha256::digest(certificate.as_bytes()))).unwrap();
        let local = std::net::TcpListener::bind(super::super::super::webauth::ADDRESS).unwrap();
        local.set_nonblocking(true).unwrap();
        let helper = tokio::spawn(super::super::super::webauth::test_service(
            root.path().to_owned(),
            local,
        ));
        let client = reqwest::Client::builder()
            .no_proxy()
            .tls_built_in_root_certs(false)
            .add_root_certificate(reqwest::Certificate::from_pem(certificate.as_bytes()).unwrap())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let local_base = "http://127.0.0.1:9844";
        let mut available = false;
        for _ in 0..100 {
            if client
                .get(format!("{local_base}/health"))
                .header("origin", &base)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                available = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(available);
        let accepted = database
            .accept_operation(
                "bootstrap-admin",
                "fixture",
                "https-summary",
                &"a".repeat(64),
                None,
            )
            .await
            .unwrap()
            .operation;
        let running = database
            .transition(
                &accepted.id,
                super::super::super::wire::OperationState::Running,
                accepted.progress,
                None,
                None,
            )
            .await
            .unwrap();
        let complete = database
            .transition(
                &running.id,
                super::super::super::wire::OperationState::Succeeded,
                running.progress,
                Some(serde_json::json!({"evidence":"x".repeat(2*1024*1024)})),
                None,
            )
            .await
            .unwrap();
        let response = client
            .get(format!("{base}{API_BASE}/operations?view=summary"))
            .bearer_auth("fixture-secret")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let summary: serde_json::Value = response.json().await.unwrap();
        assert!(serde_json::to_vec(&summary).unwrap().len() < 8192);
        let projected = summary["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|operation| operation["id"] == complete.id)
            .unwrap();
        assert!(projected["result"].is_null());
        let detail: super::super::super::wire::OperationDocument = client
            .get(format!("{base}{API_BASE}/operations/{}", complete.id))
            .bearer_auth("fixture-secret")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(detail, complete);
        assert_eq!(
            client
                .get(format!("{base}{API_BASE}/operations?view=summary"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            client
                .get(format!(
                    "{base}{API_BASE}/operations?view=summary&other=true"
                ))
                .bearer_auth("fixture-secret")
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        if browser {
            let result = tokio::time::timeout(
                Duration::from_secs(45),
                tokio::process::Command::new("node")
                    .arg("web/tests/auth-flow.mjs")
                    .arg(&base)
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                result.status.success(),
                "browser flow failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(
                tokio::time::timeout(Duration::from_secs(5), helper)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
            );
            handle.graceful_shutdown(Some(Duration::from_secs(1)));
            server.await.unwrap();
            database.shutdown().unwrap();
            return;
        }
        assert_eq!(
            client
                .get(format!("{local_base}/health"))
                .header("origin", "https://evil.example")
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            client
                .get(format!("{local_base}/health"))
                .header("host", "evil.example")
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        let local_start = client
            .get(format!("{local_base}/auth/start"))
            .send()
            .await
            .unwrap();
        let local_cookie = local_start.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let begin = client
            .get(local_start.headers()[header::LOCATION].to_str().unwrap())
            .send()
            .await
            .unwrap();
        let browser_cookie = begin.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let approve_url = begin.headers()[header::LOCATION].to_str().unwrap();
        assert_eq!(client.get(approve_url).send().await.unwrap().status(), 403);
        let approval = client
            .get(approve_url)
            .header("cookie", local_cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(approval.status(), 303);
        let finish_url = approval.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(!finish_url.contains(&token));
        assert_eq!(client.get(&finish_url).send().await.unwrap().status(), 401);
        let finish = client
            .get(&finish_url)
            .header("cookie", &browser_cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(finish.status(), 200);
        let session = finish.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        assert!(!session.contains(&token));
        let me: serde_json::Value = client
            .get(format!("{base}{API_BASE}/web-session"))
            .header("cookie", &session)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(me["scopes"], serde_json::json!(["analytics:read"]));
        let csrf = me["csrf"].as_str().unwrap();
        assert_eq!(
            client
                .get(format!("{base}{API_BASE}/analytics"))
                .header("cookie", &session)
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            client
                .get(format!("{base}{API_BASE}/tokens"))
                .header("cookie", &session)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            client
                .get(format!("{base}/openai/no-model/v1/models"))
                .header("cookie", &session)
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            client
                .delete(format!("{base}{API_BASE}/web-session"))
                .header("cookie", &session)
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            client
                .delete(format!("{base}{API_BASE}/web-session"))
                .header("cookie", &session)
                .header("x-sparkplane-csrf", csrf)
                .header("origin", "https://evil.example")
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(5), helper)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
        );
        assert!(
            client
                .get(format!("{local_base}/health"))
                .send()
                .await
                .is_err()
        );
        database
            .revoke_token("bootstrap-admin", "fixture-revoke", &created.token.id)
            .await
            .unwrap();
        state.store_auth(database.auth_snapshot().await.unwrap());
        assert_eq!(
            client
                .get(format!("{base}{API_BASE}/web-session"))
                .header("cookie", &session)
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        handle.graceful_shutdown(Some(Duration::from_secs(1)));
        server.await.unwrap();
        database.shutdown().unwrap();
    }

    #[tokio::test]
    async fn login_requires_browser_binding_and_consumes_code_once() {
        let state = AgentState::new("fixture-secret", vec!["localhost".into()], vec![]);
        let app = super::super::router(state);
        let start = app.clone().oneshot(Request::builder()
            .uri("/panel/auth/begin?intent=1111111111111111111111111111111111111111111111111111111111111111")
            .header("host", "localhost").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(start.status(), StatusCode::SEE_OTHER);
        let cookie = start.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let callback =
            reqwest::Url::parse(start.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        let flow = callback
            .query_pairs()
            .find(|(k, _)| k == "flow")
            .unwrap()
            .1
            .into_owned();
        let approval = app.clone().oneshot(Request::builder().method("POST")
            .uri("/api/sparkplane/v1/web-auth/approve").header("authorization", "Bearer fixture-secret")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"flow":flow,"intent":"1111111111111111111111111111111111111111111111111111111111111111"}).to_string())).unwrap()).await.unwrap();
        assert_eq!(approval.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(approval.into_body(), 4096)
            .await
            .unwrap();
        let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let finish = format!(
            "/panel/auth/finish?flow={flow}&code={}",
            data["code"].as_str().unwrap()
        );
        let missing = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&finish)
                    .header("host", "localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        let ok = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&finish)
                    .header("host", "localhost")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        assert!(
            ok.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .contains("HttpOnly; Secure; SameSite=Strict")
        );
        let replay = app
            .oneshot(
                Request::builder()
                    .uri(&finish)
                    .header("host", "localhost")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
    }
}
