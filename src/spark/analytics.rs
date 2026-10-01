//! Content-free analytics, retained by the existing bounded database actor.
use super::{
    economics::{Compression, Price},
    state::{DbActor, StateError},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

const MIGRATION: &str = r#"
CREATE TABLE panel_token_scopes (token_id TEXT PRIMARY KEY REFERENCES token_metadata(id) ON DELETE CASCADE, analytics_read INTEGER NOT NULL CHECK(analytics_read=1));
CREATE TABLE panel_requests (
 id TEXT PRIMARY KEY, token_id TEXT NOT NULL, session_id TEXT, model TEXT NOT NULL,
 instance TEXT NOT NULL, generation INTEGER NOT NULL, integration TEXT NOT NULL,
 protocol TEXT NOT NULL, started_at TEXT NOT NULL, finished_at TEXT,
 input_tokens INTEGER, output_tokens INTEGER, outcome TEXT NOT NULL DEFAULT 'pending',
 accounting_error INTEGER NOT NULL DEFAULT 0, duration_ms INTEGER, ttft_ms INTEGER
);
CREATE INDEX panel_requests_time ON panel_requests(started_at);
CREATE TABLE panel_usage_daily (
 day TEXT NOT NULL, model TEXT NOT NULL, instance TEXT NOT NULL, token_id TEXT NOT NULL,
 integration TEXT NOT NULL, session_id TEXT NOT NULL, generation INTEGER NOT NULL, protocol TEXT NOT NULL,
 requests INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
 unknown_usage_requests INTEGER NOT NULL DEFAULT 0, pending_requests INTEGER NOT NULL DEFAULT 0,
 failed_requests INTEGER NOT NULL DEFAULT 0, accounting_errors INTEGER NOT NULL DEFAULT 0,
 long_context_input_tokens INTEGER NOT NULL DEFAULT 0, long_context_output_tokens INTEGER NOT NULL DEFAULT 0,
 duration_ms INTEGER NOT NULL DEFAULT 0, duration_samples INTEGER NOT NULL DEFAULT 0,
 ttft_ms INTEGER NOT NULL DEFAULT 0, ttft_samples INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(day,model,instance,token_id,integration,session_id,generation,protocol)
);
CREATE TABLE panel_session_reports (session_id TEXT PRIMARY KEY REFERENCES launch_sessions(id) ON DELETE CASCADE, report_json TEXT NOT NULL);
CREATE TABLE panel_rtk_daily (session_id TEXT PRIMARY KEY,day TEXT NOT NULL,model TEXT NOT NULL,integration TEXT NOT NULL,instance TEXT NOT NULL,token_id TEXT NOT NULL,report_json TEXT NOT NULL);
CREATE INDEX panel_rtk_day ON panel_rtk_daily(day);
CREATE TABLE panel_health (resolution TEXT NOT NULL, bucket INTEGER NOT NULL, sample_json TEXT NOT NULL, PRIMARY KEY(resolution,bucket));
"#;

pub(crate) fn migration_needed(db: &Connection) -> Result<bool, StateError> {
    db.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM sqlite_master WHERE name='panel_migrations')",
        [],
        |r| r.get(0),
    )
    .map_err(sql)
}
pub(crate) fn migrate(db: &mut Connection) -> Result<(), StateError> {
    let checksum = format!("{:x}", Sha256::digest(MIGRATION));
    if !migration_needed(db)? {
        let versions: Vec<(u32, String)> = db
            .prepare("SELECT version,checksum FROM panel_migrations ORDER BY version")
            .map_err(sql)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql)?
            .collect::<Result<_, _>>()
            .map_err(sql)?;
        if versions != [(1, checksum)] {
            return Err(StateError::Unavailable(
                "unsupported or modified panel migration".into(),
            ));
        }
        return Ok(());
    }
    let tx = db.transaction().map_err(sql)?;
    tx.execute_batch(
        "CREATE TABLE panel_migrations(version INTEGER PRIMARY KEY,checksum TEXT NOT NULL);",
    )
    .map_err(sql)?;
    tx.execute_batch(MIGRATION).map_err(sql)?;
    tx.execute("INSERT INTO panel_migrations VALUES(1,?1)", [checksum])
        .map_err(sql)?;
    tx.execute_batch("INSERT INTO panel_requests(id,token_id,session_id,model,instance,generation,integration,protocol,started_at,finished_at,input_tokens,output_tokens,outcome,accounting_error) SELECT r.id,s.token_id,s.id,s.model,r.instance,0,s.integration,'historical',r.started_at,r.finished_at,r.input_tokens,r.output_tokens,r.outcome,r.accounting_error FROM launch_requests r JOIN launch_sessions s ON r.session_id=s.id;") .map_err(sql)?;
    tx.execute_batch("INSERT INTO panel_usage_daily(day,model,instance,token_id,integration,session_id,generation,protocol,requests,input_tokens,output_tokens,unknown_usage_requests,pending_requests,failed_requests,accounting_errors,long_context_input_tokens,long_context_output_tokens) SELECT substr(started_at,1,10),model,instance,token_id,integration,COALESCE(session_id,''),generation,protocol,COUNT(*),COALESCE(SUM(input_tokens),0),COALESCE(SUM(output_tokens),0),SUM(input_tokens IS NULL),SUM(finished_at IS NULL),SUM(outcome NOT IN ('succeeded','pending')),SUM(accounting_error),COALESCE(SUM(CASE WHEN input_tokens>272000 THEN input_tokens ELSE 0 END),0),COALESCE(SUM(CASE WHEN input_tokens>272000 THEN output_tokens ELSE 0 END),0) FROM panel_requests GROUP BY 1,2,3,4,5,6,7,8;").map_err(sql)?;
    tx.commit().map_err(sql)
}
fn sql(error: rusqlite::Error) -> StateError {
    StateError::Unavailable(format!("panel database: {error}"))
}

// Meters belong to the previous agent process and cannot complete after restart.
// Retain unknown usage and close their pending accounting exactly once.
pub(crate) fn interrupt_pending(db: &mut Connection) -> Result<(), StateError> {
    let tx = db.transaction().map_err(sql)?;
    tx.execute_batch("UPDATE panel_usage_daily SET failed_requests=failed_requests+pending_requests,accounting_errors=accounting_errors+pending_requests,pending_requests=0 WHERE pending_requests>0;") .map_err(sql)?;
    let now = super::state::now();
    for table in ["panel_requests", "launch_requests"] {
        tx.execute(
            &format!("UPDATE {table} SET finished_at=?1,outcome='interrupted',accounting_error=1 WHERE finished_at IS NULL"),
            [&now],
        ).map_err(sql)?;
    }
    tx.commit().map_err(sql)
}

pub(crate) enum Command {
    Query {
        kind: QueryKind,
        query: AnalyticsQuery,
        reply: oneshot::Sender<Result<Value, StateError>>,
    },
    Compression {
        actor: String,
        session: String,
        report: Compression,
        reply: oneshot::Sender<Result<Value, StateError>>,
    },
    Health {
        sample: Value,
        reply: oneshot::Sender<Result<(), StateError>>,
    },
    Cleanup {
        reply: oneshot::Sender<Result<(), StateError>>,
    },
}
pub(crate) enum QueryKind {
    Summary,
    Operations,
    Requests,
    Sessions,
    Audit,
    Health,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnalyticsQuery {
    pub days: u32,
    pub offset: u32,
    pub limit: u32,
    pub model: Option<String>,
    pub instance: Option<String>,
    pub token: Option<String>,
    pub integration: Option<String>,
    pub session: Option<String>,
    pub previous: bool,
}
impl Default for AnalyticsQuery {
    fn default() -> Self {
        Self {
            days: 7,
            offset: 0,
            limit: 50,
            model: None,
            instance: None,
            token: None,
            integration: None,
            session: None,
            previous: false,
        }
    }
}
impl AnalyticsQuery {
    pub fn validate(&self) -> Result<(), StateError> {
        if !(1..=365).contains(&self.days)
            || !(1..=200).contains(&self.limit)
            || self.offset > 100000
            || [
                &self.model,
                &self.instance,
                &self.token,
                &self.integration,
                &self.session,
            ]
            .into_iter()
            .flatten()
            .any(|s| s.len() > 256)
        {
            return Err(StateError::Invalid(
                "invalid analytics range or pagination".into(),
            ));
        }
        Ok(())
    }
    fn dates(&self) -> (String, String) {
        let end = chrono::Utc::now().date_naive() + chrono::Duration::days(1)
            - chrono::Duration::days(if self.previous {
                i64::from(self.days)
            } else {
                0
            });
        let start = end - chrono::Duration::days(i64::from(self.days));
        (start.to_string(), end.to_string())
    }
}
impl DbActor {
    pub(crate) async fn list_operation_summaries(
        &self,
    ) -> Result<Vec<super::wire::OperationDocument>, StateError> {
        let mut summary = self
            .panel_query(QueryKind::Operations, AnalyticsQuery::default())
            .await?;
        serde_json::from_value(summary["operations"].take())
            .map_err(|_| StateError::Unavailable("invalid operation metadata".into()))
    }
    pub(crate) async fn panel_query(
        &self,
        kind: QueryKind,
        query: AnalyticsQuery,
    ) -> Result<Value, StateError> {
        query.validate()?;
        let (reply, receive) = oneshot::channel();
        self.panel_command(Command::Query { kind, query, reply })?;
        receive
            .await
            .map_err(|_| StateError::Unavailable("panel database unavailable".into()))?
    }
    pub async fn session_compression(
        &self,
        actor: &str,
        session: &str,
        report: Compression,
    ) -> Result<Value, StateError> {
        let (reply, receive) = oneshot::channel();
        self.panel_command(Command::Compression {
            actor: actor.into(),
            session: session.into(),
            report,
            reply,
        })?;
        receive
            .await
            .map_err(|_| StateError::Unavailable("panel database unavailable".into()))?
    }
    pub(crate) async fn panel_health(&self, sample: Value) -> Result<(), StateError> {
        let (reply, receive) = oneshot::channel();
        self.panel_command(Command::Health { sample, reply })?;
        receive
            .await
            .map_err(|_| StateError::Unavailable("panel database unavailable".into()))?
    }
    pub(crate) async fn panel_cleanup(&self) -> Result<(), StateError> {
        let (reply, receive) = oneshot::channel();
        self.panel_command(Command::Cleanup { reply })?;
        receive
            .await
            .map_err(|_| StateError::Unavailable("panel database unavailable".into()))?
    }
}
pub(crate) fn begin(
    db: &Connection,
    id: &str,
    token: &str,
    instance: &str,
    protocol: &str,
) -> Result<(), StateError> {
    let session: Option<(String,String,String)> = db.query_row("SELECT id,model,integration FROM launch_sessions WHERE token_id=?1 AND finished_at IS NULL",[token],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
    let document = super::state::read_instance(db, instance).ok();
    let model = document
        .as_ref()
        .map(|d| d.model.clone())
        .or_else(|| session.as_ref().map(|s| s.1.clone()))
        .unwrap_or_else(|| instance.into());
    let generation = i64::try_from(document.as_ref().map(|d| d.generation).unwrap_or(0))
        .map_err(|_| StateError::Invalid("generation overflow".into()))?;
    let integration = session.as_ref().map(|s| s.2.as_str()).unwrap_or("api");
    let now = super::state::now();
    db.execute("INSERT INTO panel_requests(id,token_id,session_id,model,instance,generation,integration,protocol,started_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id,token,session.as_ref().map(|s|&s.0),model,instance,generation,integration,protocol,now]).map_err(sql)?;
    db.execute("INSERT INTO panel_usage_daily(day,model,instance,token_id,integration,session_id,generation,protocol,requests,unknown_usage_requests,pending_requests) VALUES(substr(?1,1,10),?2,?3,?4,?5,?6,?7,?8,1,1,1) ON CONFLICT DO UPDATE SET requests=requests+1,unknown_usage_requests=unknown_usage_requests+1,pending_requests=pending_requests+1",params![now,model,instance,token,integration,session.as_ref().map(|s|s.0.as_str()).unwrap_or(""),generation,protocol]).map_err(sql)?;
    Ok(())
}
pub(crate) fn record(
    db: &Connection,
    id: &str,
    counts: Option<(i64, i64)>,
    outcome: &str,
    incomplete: bool,
    duration_ms: Option<u64>,
    ttft_ms: Option<u64>,
) -> Result<(), StateError> {
    let changed=db.execute("UPDATE panel_requests SET input_tokens=?2,output_tokens=?3,outcome=?4,accounting_error=?5,finished_at=?6,duration_ms=?7,ttft_ms=?8 WHERE id=?1 AND finished_at IS NULL",params![id,counts.map(|c|c.0),counts.map(|c|c.1),outcome,incomplete,super::state::now(),duration_ms.map(|v|v.min(i64::MAX as u64) as i64),ttft_ms.map(|v|v.min(i64::MAX as u64) as i64)]).map_err(sql)?;
    if changed == 0 {
        return Ok(());
    }
    db.execute("UPDATE panel_usage_daily SET input_tokens=input_tokens+?2,output_tokens=output_tokens+?3,unknown_usage_requests=unknown_usage_requests-?4,pending_requests=pending_requests-1,failed_requests=failed_requests+?5,accounting_errors=accounting_errors+?6,long_context_input_tokens=long_context_input_tokens+?7,long_context_output_tokens=long_context_output_tokens+?8,duration_ms=duration_ms+?9,duration_samples=duration_samples+?10,ttft_ms=ttft_ms+?11,ttft_samples=ttft_samples+?12 WHERE (day,model,instance,token_id,integration,session_id,generation,protocol)=(SELECT substr(started_at,1,10),model,instance,token_id,integration,COALESCE(session_id,''),generation,protocol FROM panel_requests WHERE id=?1)",params![id,counts.map(|c|c.0).unwrap_or(0),counts.map(|c|c.1).unwrap_or(0),counts.is_some(),outcome!="succeeded",incomplete,counts.filter(|c|c.0>272000).map(|c|c.0).unwrap_or(0),counts.filter(|c|c.0>272000).map(|c|c.1).unwrap_or(0),duration_ms.unwrap_or(0).min(i64::MAX as u64) as i64,duration_ms.is_some(),ttft_ms.unwrap_or(0).min(i64::MAX as u64) as i64,ttft_ms.is_some()]).map_err(sql)?;
    Ok(())
}
pub(crate) fn dispatch(db: &mut Connection, command: Command) {
    match command {
        Command::Query { kind, query, reply } => {
            let _ = reply.send(query_data(db, kind, query));
        }
        Command::Compression {
            actor,
            session,
            report,
            reply,
        } => {
            let _ = reply.send(compression(db, &actor, &session, report));
        }
        Command::Health { sample, reply } => {
            let _ = reply.send(store_health(db, &sample));
        }
        Command::Cleanup { reply } => {
            let _ = reply.send(cleanup(db));
        }
    }
}
fn rows(
    db: &Connection,
    sql_text: &str,
    parameters: impl rusqlite::Params,
) -> Result<Vec<Value>, StateError> {
    let mut statement = db.prepare(sql_text).map_err(sql)?;
    let names = statement
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    statement
        .query_map(parameters, |row| {
            let mut object = serde_json::Map::new();
            for (i, name) in names.iter().enumerate() {
                let value = match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(v) => json!(v),
                    rusqlite::types::ValueRef::Real(v) => json!(v),
                    rusqlite::types::ValueRef::Text(v) => json!(String::from_utf8_lossy(v)),
                    rusqlite::types::ValueRef::Blob(_) => Value::Null,
                };
                object.insert(name.clone(), value);
            }
            Ok(Value::Object(object))
        })
        .map_err(sql)?
        .collect::<Result<_, _>>()
        .map_err(sql)
}
const FILTER: &str = "day>=?1 AND day<?2 AND (?3 IS NULL OR model=?3) AND (?4 IS NULL OR instance=?4) AND (?5 IS NULL OR token_id=?5) AND (?6 IS NULL OR integration=?6) AND (?7 IS NULL OR session_id=?7)";
const SUMS: &str = "COALESCE(SUM(requests),0) requests,COALESCE(SUM(input_tokens),0) input_tokens,COALESCE(SUM(output_tokens),0) output_tokens,COALESCE(SUM(unknown_usage_requests),0) unknown_usage_requests,COALESCE(SUM(pending_requests),0) pending_requests,COALESCE(SUM(failed_requests),0) failed_requests,COALESCE(SUM(accounting_errors),0) accounting_errors,COALESCE(SUM(long_context_input_tokens),0) long_context_input_tokens,COALESCE(SUM(long_context_output_tokens),0) long_context_output_tokens,CAST(SUM(duration_ms) AS REAL)/NULLIF(SUM(duration_samples),0) mean_duration_ms,CAST(SUM(ttft_ms) AS REAL)/NULLIF(SUM(ttft_samples),0) mean_ttft_ms,1000.0*SUM(output_tokens)/NULLIF(SUM(duration_ms),0) output_tokens_per_second";
fn query_data(
    db: &Connection,
    kind: QueryKind,
    query: AnalyticsQuery,
) -> Result<Value, StateError> {
    query.validate()?;
    let (start, end) = query.dates();
    let parameters = params![
        start,
        end,
        query.model,
        query.instance,
        query.token,
        query.integration,
        query.session
    ];
    match kind {
        QueryKind::Operations => {
            // Qualification evidence can be megabytes per operation. Select
            // list metadata directly without loading or decoding result_json.
            let mut operations = rows(
                db,
                "SELECT id,kind,actor_token_id,target,state,progress_json progress,created_at,updated_at,NULL result,problem_json problem FROM operations ORDER BY created_at DESC,id DESC LIMIT 256",
                [],
            )?;
            for operation in &mut operations {
                operation["schema"] = json!(super::wire::OPERATION_SCHEMA);
                for key in ["progress", "problem"] {
                    if let Some(text) = operation[key].as_str() {
                        operation[key] = serde_json::from_str(text).map_err(|_| {
                            StateError::Unavailable("invalid operation metadata".into())
                        })?;
                    }
                }
            }
            Ok(json!({"schema":super::wire::OPERATION_LIST_SCHEMA,"operations":operations}))
        }
        QueryKind::Summary => {
            let catalog =
                super::economics::catalog().map_err(|e| StateError::Unavailable(e.to_string()))?;
            let mut summary = rows(
                db,
                &format!("SELECT {SUMS} FROM panel_usage_daily WHERE {FILTER}"),
                parameters,
            )?
            .remove(0);
            // Price each actual model before aggregation. Never apply one model's
            // tariff to another model, or silently present an unpriced subtotal.
            let models = rows(
                db,
                &format!(
                    "SELECT model,{SUMS} FROM panel_usage_daily WHERE {FILTER} GROUP BY model ORDER BY requests DESC,model"
                ),
                parameters,
            )?;
            let mut equivalent = Some(0u64);
            let mut unpriced_requests = 0u64;
            let mut comparisons = Vec::new();
            for usage in &models {
                let model = usage["model"].as_str().unwrap_or_default();
                let price = catalog.models.iter().find(|p| p.matches_model(model));
                let value = price.and_then(|p| cost(usage, p));
                equivalent = equivalent
                    .zip(value)
                    .and_then(|(total, value)| total.checked_add(value));
                if price.is_none() {
                    unpriced_requests =
                        unpriced_requests.saturating_add(usage["requests"].as_u64().unwrap_or(0));
                }
                if comparisons.len() < 200 {
                    comparisons.push(json!({"model":model,"cloud_model":price.map(|p|&p.model),"provider":price.and_then(|p|p.provider.as_ref()),"source":price.map(|p|&p.source),"verified_at":price.map(|p|&p.verified_at),"cloud_equivalent_usd_nanos":value}));
                }
            }
            let series = rows(
                db,
                &format!(
                    "SELECT day,{SUMS} FROM panel_usage_daily WHERE {FILTER} GROUP BY day ORDER BY day"
                ),
                parameters,
            )?;
            let mut breakdown = serde_json::Map::new();
            for key in [
                "model",
                "instance",
                "token_id",
                "integration",
                "session_id",
                "protocol",
            ] {
                breakdown.insert(key.into(),json!(rows(db,&format!("SELECT {key},{SUMS} FROM panel_usage_daily WHERE {FILTER} GROUP BY {key} ORDER BY requests DESC LIMIT 200"),parameters)?));
            }
            let reports = rows(
                db,
                &format!("SELECT model,report_json FROM panel_rtk_daily WHERE {FILTER}"),
                parameters,
            )?;
            let mut compression = Compression {
                status: "active".into(),
                ..Default::default()
            };
            let mut covered = 0;
            let mut unpriced_sessions = 0;
            let mut saved_tokens = 0u64;
            let mut rtk_value = Some(0u64);
            for row in &reports {
                if let Some(text) = row["report_json"].as_str()
                    && let Ok(report) = serde_json::from_str::<Compression>(text)
                    && report.status == "active"
                    && report.metrics_errors == 0
                {
                    compression.raw_bytes = compression.raw_bytes.saturating_add(report.raw_bytes);
                    compression.filtered_bytes = compression
                        .filtered_bytes
                        .saturating_add(report.filtered_bytes);
                    compression.commands = compression.commands.saturating_add(report.commands);
                    covered += 1;
                    saved_tokens = saved_tokens.saturating_add(report.saved_tokens());
                    let price = catalog
                        .models
                        .iter()
                        .find(|p| p.matches_model(row["model"].as_str().unwrap_or_default()));
                    if price.is_none() {
                        unpriced_sessions += 1;
                    }
                    let value = price.and_then(|p| {
                        u64::try_from(
                            u128::from(report.saved_tokens())
                                * u128::from(p.input_microusd_per_million)
                                / 1000,
                        )
                        .ok()
                    });
                    rtk_value = rtk_value
                        .zip(value)
                        .and_then(|(total, value)| total.checked_add(value));
                }
            }
            summary["comparisons"] = json!(comparisons);
            summary["cloud_equivalent_usd_nanos"] = json!(equivalent);
            summary["unpriced_requests"] = json!(unpriced_requests);
            if covered == 0 {
                rtk_value = None;
            }
            Ok(
                json!({"schema":"sparkplane.analytics/v1","from":start,"to":end,"summary":summary,"series":series,"breakdown":breakdown,"catalog":catalog,"rtk":{"client_reported":true,"reported_sessions":reports.len(),"covered_sessions":covered,"unpriced_sessions":unpriced_sessions,"commands":compression.commands,"raw_bytes":compression.raw_bytes,"filtered_bytes":compression.filtered_bytes,"estimated_saved_tokens":saved_tokens,"estimated_saved_usd_nanos":rtk_value},"coverage":{"request_detail_days":90,"rollup_days":365,"historical_clients":"launch sessions only before panel upgrade"}}),
            )
        }
        QueryKind::Requests => {
            let filter = FILTER.replace("day", "substr(started_at,1,10)");
            let items = rows(
                db,
                &format!(
                    "SELECT * FROM panel_requests WHERE {filter} ORDER BY started_at DESC,id DESC LIMIT ?8 OFFSET ?9"
                ),
                params![
                    start,
                    end,
                    query.model,
                    query.instance,
                    query.token,
                    query.integration,
                    query.session,
                    query.limit + 1,
                    query.offset
                ],
            )?;
            Ok(page(items, &query))
        }
        QueryKind::Sessions => {
            let items = rows(
                db,
                "SELECT s.id,s.token_id,s.instance,s.model,s.integration,s.eco_mode,s.started_at,s.finished_at,p.report_json compression_json FROM launch_sessions s LEFT JOIN panel_session_reports p ON p.session_id=s.id WHERE substr(s.started_at,1,10)>=?1 AND substr(s.started_at,1,10)<?2 AND (?3 IS NULL OR s.model=?3) AND (?4 IS NULL OR s.instance=?4) AND (?5 IS NULL OR s.token_id=?5) AND (?6 IS NULL OR s.integration=?6) AND (?7 IS NULL OR s.id=?7) ORDER BY s.started_at DESC,s.id DESC LIMIT ?8 OFFSET ?9",
                params![
                    start,
                    end,
                    query.model,
                    query.instance,
                    query.token,
                    query.integration,
                    query.session,
                    query.limit + 1,
                    query.offset
                ],
            )?;
            Ok(page(items, &query))
        }
        QueryKind::Audit => Ok(page(
            rows(
                db,
                "SELECT sequence,occurred_at,actor_token_id,action,target,outcome,metadata_json FROM audit WHERE substr(occurred_at,1,10)>=?1 AND substr(occurred_at,1,10)<?2 ORDER BY sequence DESC LIMIT ?3 OFFSET ?4",
                params![start, end, query.limit + 1, query.offset],
            )?,
            &query,
        )),
        QueryKind::Health => {
            let resolution = if query.days == 1 {
                "raw"
            } else if query.days <= 30 {
                "minute"
            } else {
                "hour"
            };
            let after = (chrono::Utc::now() - chrono::Duration::days(i64::from(query.days)))
                .timestamp_millis();
            let width = match resolution {
                "raw" => 5000,
                "minute" => 60000,
                _ => 3600000,
            };
            let stride =
                (i64::from(query.days) * 86400000 / i64::from(query.limit) / width).max(1) * width;
            // Group only timestamp keys. Sorting every full telemetry document
            // can spill outside the confined agent's writable paths as history
            // grows; fetch at most one bounded page of documents afterwards.
            let mut items = rows(
                db,
                "SELECT MAX(bucket) bucket FROM panel_health WHERE resolution=?1 AND bucket>=?2 GROUP BY bucket/?3 ORDER BY bucket DESC LIMIT ?4 OFFSET ?5",
                params![resolution, after, stride, query.limit + 1, query.offset],
            )?;
            let mut samples = db
                .prepare("SELECT sample_json FROM panel_health WHERE resolution=?1 AND bucket=?2")
                .map_err(sql)?;
            for item in &mut items {
                let sample: String = samples
                    .query_row(params![resolution, item["bucket"].as_i64()], |row| {
                        row.get(0)
                    })
                    .map_err(sql)?;
                item["sample_json"] = json!(sample);
            }
            Ok(page(items, &query))
        }
    }
}
fn page(mut items: Vec<Value>, query: &AnalyticsQuery) -> Value {
    let next = (items.len() > query.limit as usize).then_some(query.offset + query.limit);
    items.truncate(query.limit as usize);
    json!({"items":items,"next_offset":next})
}
pub(crate) fn cost(usage: &Value, price: &Price) -> Option<u64> {
    let requests = usage["requests"].as_u64()?;
    if requests > 0 && usage["unknown_usage_requests"].as_u64()? == requests {
        return None;
    }
    if price
        .long_context_threshold
        .is_some_and(|threshold| threshold != 272000)
    {
        return None;
    }
    let input = u128::from(usage["input_tokens"].as_u64()?);
    let output = u128::from(usage["output_tokens"].as_u64()?);
    let long_input = u128::from(usage["long_context_input_tokens"].as_u64()?);
    let long_output = u128::from(usage["long_context_output_tokens"].as_u64()?);
    let value = (input.checked_sub(long_input)? * u128::from(price.input_microusd_per_million))
        + (output.checked_sub(long_output)? * u128::from(price.output_microusd_per_million))
        + long_input
            * u128::from(
                price
                    .long_input_microusd_per_million
                    .unwrap_or(price.input_microusd_per_million),
            )
        + long_output
            * u128::from(
                price
                    .long_output_microusd_per_million
                    .unwrap_or(price.output_microusd_per_million),
            );
    u64::try_from(value / 1000).ok()
}
fn compression(
    db: &mut Connection,
    actor: &str,
    session: &str,
    report: Compression,
) -> Result<Value, StateError> {
    if ![
        "active",
        "unavailable",
        "disabled",
        "inactive",
        "inactive (inherited permissions)",
        "inactive (hook unavailable or awaiting trust)",
    ]
    .contains(&report.status.as_str())
        || report.raw_bytes > i64::MAX as u64
        || report.filtered_bytes > i64::MAX as u64
    {
        return Err(StateError::Invalid(
            "invalid aggregate compression report".into(),
        ));
    }
    let text =
        serde_json::to_string(&report).map_err(|_| StateError::Invalid("invalid report".into()))?;
    if let Some(old) = db
        .query_row(
            "SELECT report_json FROM panel_session_reports WHERE session_id=?1",
            [session],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(sql)?
    {
        if old == text {
            return Ok(json!({"stored":true}));
        }
        return Err(StateError::Conflict(
            "compression report already finalized".into(),
        ));
    }
    let tx = db.transaction().map_err(sql)?;
    let (model,integration,instance,token,finished):(String,String,String,String,Option<String>)=tx.query_row("SELECT model,integration,instance,token_id,finished_at FROM launch_sessions WHERE id=?1",[session],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(sql)?.ok_or(StateError::NotFound)?;
    let finished = finished.ok_or_else(|| {
        StateError::Conflict("finish session before submitting compression".into())
    })?;
    tx.execute(
        "INSERT INTO panel_session_reports VALUES(?1,?2)",
        params![session, text],
    )
    .map_err(sql)?;
    tx.execute(
        "INSERT INTO panel_rtk_daily VALUES(?1,substr(?2,1,10),?3,?4,?5,?6,?7)",
        params![session, finished, model, integration, instance, token, text],
    )
    .map_err(sql)?;
    tx.execute("INSERT INTO audit(occurred_at,actor_token_id,action,target,outcome,metadata_json) VALUES(?1,?2,'launch-session.compression',?3,'succeeded','{}')",params![super::state::now(),actor,session]).map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(json!({"stored":true}))
}
fn store_health(db: &mut Connection, sample: &Value) -> Result<(), StateError> {
    let time = sample["observed_at_unix_ms"]
        .as_u64()
        .ok_or_else(|| StateError::Invalid("health sample lacks time".into()))?;
    let text = serde_json::to_string(sample)
        .map_err(|_| StateError::Invalid("invalid health sample".into()))?;
    let tx = db.transaction().map_err(sql)?;
    for (resolution, width) in [("raw", 5000), ("minute", 60000), ("hour", 3600000)] {
        tx.execute("INSERT INTO panel_health VALUES(?1,?2,?3) ON CONFLICT DO UPDATE SET sample_json=excluded.sample_json",params![resolution,(time/width*width) as i64,text]).map_err(sql)?;
    }
    tx.commit().map_err(sql)
}
fn cleanup(db: &mut Connection) -> Result<(), StateError> {
    let tx = db.transaction().map_err(sql)?;
    tx.execute_batch("DELETE FROM panel_requests WHERE started_at<strftime('%Y-%m-%d','now','-90 days'); DELETE FROM launch_sessions WHERE started_at<strftime('%Y-%m-%d','now','-90 days'); DELETE FROM panel_usage_daily WHERE day<strftime('%Y-%m-%d','now','-365 days'); DELETE FROM panel_rtk_daily WHERE day<strftime('%Y-%m-%d','now','-365 days'); DELETE FROM panel_health WHERE (resolution='raw' AND bucket<(unixepoch('now')-86400)*1000) OR (resolution='minute' AND bucket<(unixepoch('now')-2592000)*1000) OR (resolution='hour' AND bucket<(unixepoch('now')-31536000)*1000);").map_err(sql)?;
    tx.commit().map_err(sql)
}

#[cfg(test)]
mod tests {
    use super::super::{sessions::UsageMeter, upstream::GenerationEvent};
    use super::*;

    #[tokio::test]
    async fn operation_summaries_omit_large_results_and_preserve_details() {
        use super::super::wire::OperationState;
        let root = tempfile::tempdir().unwrap();
        let db = DbActor::open(
            root.path().join("state"),
            root.path().join("backups"),
            64,
            2,
            secrecy::SecretString::from("fixture"),
        )
        .unwrap();
        let accepted = db
            .accept_operation(
                "admin",
                "fixture",
                "summary-fixture",
                &"a".repeat(64),
                Some("model".into()),
            )
            .await
            .unwrap()
            .operation;
        let running = db
            .transition(
                &accepted.id,
                OperationState::Running,
                accepted.progress,
                None,
                None,
            )
            .await
            .unwrap();
        let complete = db
            .transition(
                &running.id,
                OperationState::Succeeded,
                running.progress,
                Some(json!({"evidence":"x".repeat(2*1024*1024)})),
                None,
            )
            .await
            .unwrap();
        let summary = db
            .panel_query(QueryKind::Operations, AnalyticsQuery::default())
            .await
            .unwrap();
        let mut expected = serde_json::to_value(&complete).unwrap();
        expected["result"] = Value::Null;
        assert_eq!(summary["operations"], json!([expected]));
        assert!(serde_json::to_vec(&summary).unwrap().len() < 4096);
        assert_eq!(
            serde_json::to_value(db.list_operation_summaries().await.unwrap()).unwrap(),
            summary["operations"]
        );
        assert_eq!(db.operation(&complete.id).await.unwrap(), complete);
        assert_eq!(db.list_operations().await.unwrap(), vec![complete]);
        db.shutdown().unwrap();
    }

    #[tokio::test]
    async fn restart_closes_pending_accounting_and_recovery_without_changing_child_intent() {
        let root = tempfile::tempdir().unwrap();
        let open = || {
            DbActor::open(
                root.path().join("state"),
                root.path().join("backups"),
                64,
                2,
                secrecy::SecretString::from("fixture"),
            )
            .unwrap()
        };
        let db = open();
        db.begin_usage("external", "pending-request", "model", "openai.responses")
            .await
            .unwrap();
        let recovery = db
            .accept_operation(
                "external",
                "instance.recover",
                "recovery-key",
                &"0".repeat(64),
                Some("instance".into()),
            )
            .await
            .unwrap();
        let child = db
            .accept_operation(
                "external",
                "instance.stop",
                "child-key",
                &"0".repeat(64),
                Some("instance".into()),
            )
            .await
            .unwrap();
        db.shutdown().unwrap();
        let db = open();
        let summary = db
            .panel_query(QueryKind::Summary, AnalyticsQuery::default())
            .await
            .unwrap();
        assert_eq!(summary["summary"]["pending_requests"], 0);
        assert_eq!(summary["summary"]["unknown_usage_requests"], 1);
        assert_eq!(summary["summary"]["failed_requests"], 1);
        assert_eq!(summary["summary"]["accounting_errors"], 1);
        let interrupted = db.operation(&recovery.operation.id).await.unwrap();
        assert_eq!(
            interrupted.state,
            super::super::wire::OperationState::Failed
        );
        assert_eq!(
            interrupted.problem.unwrap().code,
            "spark.instance.recovery-interrupted"
        );
        assert_eq!(
            db.operation(&child.operation.id).await.unwrap().state,
            super::super::wire::OperationState::Accepted
        );
        db.shutdown().unwrap();
        let db = open();
        assert_eq!(
            db.panel_query(QueryKind::Summary, AnalyticsQuery::default())
                .await
                .unwrap()["summary"]["failed_requests"],
            1
        );
        db.shutdown().unwrap();
    }

    #[tokio::test]
    async fn native_ttft_waits_for_generated_output_and_missing_counts_stay_unknown() {
        let root = tempfile::tempdir().unwrap();
        let db = DbActor::open(
            root.path().join("state"),
            root.path().join("backups"),
            64,
            2,
            secrecy::SecretString::from("fixture"),
        )
        .unwrap();
        for token in ["metadata-only", "with-output", "missing-output-count"] {
            let meter = UsageMeter::begin_protocol(Some(&db), token, "model", "openai.responses")
                .await
                .unwrap();
            meter.raw(b"data: {\"type\":\"response.created\"}\n\n");
            if token == "with-output" {
                meter.raw(b"data: {\"type\":\"response.reasoning_text.delta\",\"delta\":\"Thinking\"}\n\n");
                meter.raw(b"data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":1}}}\n\n");
            } else if token == "missing-output-count" {
                meter.json(br#"{"usage":{"input_tokens":5}}"#, true);
            } else {
                meter.raw(b"data: {\"type\":\"response.completed\",\"response\":{}}\n\n");
            }
            drop(meter);
        }
        for _ in 0..100 {
            let data = db
                .panel_query(QueryKind::Summary, AnalyticsQuery::default())
                .await
                .unwrap();
            if data["summary"]["pending_requests"] == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let data = db
            .panel_query(QueryKind::Requests, AnalyticsQuery::default())
            .await
            .unwrap();
        for request in data["items"].as_array().unwrap() {
            if request["token_id"] == "with-output" {
                assert!(request["ttft_ms"].is_number());
                assert_eq!(request["output_tokens"], 1);
            } else {
                assert!(request["ttft_ms"].is_null());
                assert!(request["input_tokens"].is_null());
                assert!(request["output_tokens"].is_null());
            }
        }
        db.shutdown().unwrap();
    }
    #[tokio::test]
    async fn all_clients_embeddings_and_interrupted_streams_are_counted_once() {
        let root = tempfile::tempdir().unwrap();
        let db = DbActor::open(
            root.path().join("state"),
            root.path().join("backups"),
            64,
            2,
            secrecy::SecretString::from("fixture"),
        )
        .unwrap();
        let meter = UsageMeter::begin_protocol(
            Some(&db),
            "external-client",
            "embedding",
            "openai.embeddings",
        )
        .await
        .unwrap();
        meter.json(br#"{"usage":{"prompt_tokens":12,"total_tokens":12}}"#, true);
        drop(meter);
        let meter = UsageMeter::begin_protocol(Some(&db), "external-client", "chat", "openai.chat")
            .await
            .unwrap();
        meter.observe(&GenerationEvent::Usage {
            prompt_tokens: 272001,
            completion_tokens: 7,
        });
        meter.observe(&GenerationEvent::Usage {
            prompt_tokens: 272001,
            completion_tokens: 7,
        });
        meter.observe(&GenerationEvent::Done);
        drop(meter);
        let meter =
            UsageMeter::begin_protocol(Some(&db), "other-client", "chat", "openai.responses")
                .await
                .unwrap();
        drop(meter);
        let mut data = Value::Null;
        for _ in 0..100 {
            data = db
                .panel_query(QueryKind::Summary, AnalyticsQuery::default())
                .await
                .unwrap();
            if data["summary"]["pending_requests"] == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(data["summary"]["requests"], 3);
        assert_eq!(data["summary"]["input_tokens"], 272013);
        assert_eq!(data["summary"]["output_tokens"], 7);
        assert_eq!(data["summary"]["unknown_usage_requests"], 1);
        assert_eq!(data["summary"]["failed_requests"], 1);
        let filtered = db
            .panel_query(
                QueryKind::Summary,
                AnalyticsQuery {
                    token: Some("other-client".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(filtered["summary"]["comparisons"][0]["cloud_equivalent_usd_nanos"].is_null());
        assert_eq!(
            db.panel_query(
                QueryKind::Requests,
                AnalyticsQuery {
                    limit: 2,
                    ..Default::default()
                }
            )
            .await
            .unwrap()["next_offset"],
            2
        );
        db.shutdown().unwrap();
        let connection = Connection::open(root.path().join("state")).unwrap();
        let id: String = connection
            .query_row(
                "SELECT id FROM panel_requests WHERE protocol='openai.chat'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        record(
            &connection,
            &id,
            Some((99, 99)),
            "succeeded",
            false,
            Some(2),
            None,
        )
        .unwrap();
        assert_eq!(
            connection
                .query_row("SELECT SUM(input_tokens) FROM panel_usage_daily", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            272013
        );
    }
    #[test]
    fn cloud_costs_use_each_models_tariff_and_expose_unpriced_coverage() {
        let root = tempfile::tempdir().unwrap();
        let db = DbActor::open(
            root.path().join("state"),
            root.path().join("backups"),
            64,
            2,
            secrecy::SecretString::from("fixture"),
        )
        .unwrap();
        db.shutdown().unwrap();
        let connection = Connection::open(root.path().join("state")).unwrap();
        let models = [
            "huggingface:RadixArk/Qwen3.8-Flash-Next-NVFP4@revision#artifact",
            "huggingface:Qwen/Qwen3.8-27B-FP8@revision#artifact",
        ];
        for (index, model) in models.iter().enumerate() {
            connection.execute("INSERT INTO panel_usage_daily(day,model,instance,token_id,integration,session_id,generation,protocol,requests,input_tokens,output_tokens) VALUES(date('now'),?1,'fixture','token','codex',?2,1,'openai.responses',1,100,10)", params![model, index.to_string()]).unwrap();
            connection.execute("INSERT INTO panel_rtk_daily VALUES(?1,date('now'),?2,'codex','fixture','token',?3)", params![index.to_string(), model, serde_json::to_string(&Compression {status:"active".into(), commands:1, raw_bytes:400, filtered_bytes:0, metrics_errors:0}).unwrap()]).unwrap();
        }
        let summary =
            query_data(&connection, QueryKind::Summary, AnalyticsQuery::default()).unwrap();
        assert_eq!(summary["summary"]["cloud_equivalent_usd_nanos"], 89_000);
        assert_eq!(summary["summary"]["unpriced_requests"], 0);
        assert_eq!(summary["rtk"]["estimated_saved_usd_nanos"], 60_000);
        let filtered = query_data(
            &connection,
            QueryKind::Summary,
            AnalyticsQuery {
                model: Some(models[0].into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered["summary"]["cloud_equivalent_usd_nanos"], 25_000);
        connection.execute("INSERT INTO panel_usage_daily(day,model,instance,token_id,integration,session_id,generation,protocol,requests,input_tokens) VALUES(date('now'),'unpriced-model','fixture','token','codex','unpriced',1,'openai.responses',1,100)", []).unwrap();
        connection.execute("INSERT INTO panel_rtk_daily VALUES('unpriced',date('now'),'unpriced-model','codex','fixture','token',?1)", [serde_json::to_string(&Compression {status:"active".into(), commands:1, raw_bytes:400, ..Default::default()}).unwrap()]).unwrap();
        let summary =
            query_data(&connection, QueryKind::Summary, AnalyticsQuery::default()).unwrap();
        assert!(summary["summary"]["cloud_equivalent_usd_nanos"].is_null());
        assert_eq!(summary["summary"]["unpriced_requests"], 1);
        assert_eq!(summary["summary"]["requests"], 3);
        assert!(summary["rtk"]["estimated_saved_usd_nanos"].is_null());
        assert_eq!(summary["rtk"]["unpriced_sessions"], 1);
    }
    #[test]
    fn price_tiers_and_unknown_usage_remain_distinct() {
        let mut catalog = super::super::economics::catalog().unwrap();
        catalog.models[0].input_microusd_per_million = 2_000_000;
        catalog.models[0].output_microusd_per_million = 10_000_000;
        catalog.models[0].long_context_threshold = Some(272_000);
        catalog.models[0].long_input_microusd_per_million = Some(4_000_000);
        catalog.models[0].long_output_microusd_per_million = Some(15_000_000);
        let usage = json!({"requests":1,"unknown_usage_requests":0,"input_tokens":272001,"output_tokens":7,"long_context_input_tokens":272001,"long_context_output_tokens":7});
        assert_eq!(cost(&usage, &catalog.models[0]), Some(1088109000));
        let mut unknown = usage;
        unknown["unknown_usage_requests"] = json!(1);
        assert_eq!(cost(&unknown, &catalog.models[0]), None);
    }
    #[test]
    fn health_history_stays_bounded_with_large_samples() {
        // SQLite's heap limit is process-wide, so isolate this resource test.
        const CHILD: &str = "SPARKPLANE_HEALTH_MEMORY_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "spark::analytics::tests::health_history_stays_bounded_with_large_samples",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE panel_health(resolution TEXT NOT NULL,bucket INTEGER NOT NULL,sample_json TEXT NOT NULL,PRIMARY KEY(resolution,bucket)); PRAGMA temp_store=MEMORY;").unwrap();
        let latest = chrono::Utc::now().timestamp_millis() / 5000 * 5000;
        let payload = json!({"fixture":"x".repeat(8192)}).to_string();
        let transaction = db.transaction().unwrap();
        for index in 0..6000 {
            transaction
                .execute(
                    "INSERT INTO panel_health VALUES('raw',?1,?2)",
                    params![latest - index * 5000, payload],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        db.pragma_update(None, "hard_heap_limit", 96 * 1024 * 1024)
            .unwrap();
        let query = AnalyticsQuery {
            days: 1,
            ..Default::default()
        };
        let data = query_data(&db, QueryKind::Health, query).unwrap();
        let items = data["items"].as_array().unwrap();
        assert_eq!(items[0]["bucket"], latest);
        assert!(items.len() <= 50);
        for item in items {
            assert_eq!(item["sample_json"], payload);
        }
        for pair in items.windows(2) {
            assert!(pair[0]["bucket"].as_i64().unwrap() > pair[1]["bucket"].as_i64().unwrap());
        }
    }
    #[test]
    fn retention_preserves_daily_rollups_and_migration_identity() {
        let root = tempfile::tempdir().unwrap();
        let db = DbActor::open(
            root.path().join("state"),
            root.path().join("backups"),
            64,
            2,
            secrecy::SecretString::from("fixture"),
        )
        .unwrap();
        db.shutdown().unwrap();
        let mut connection = Connection::open(root.path().join("state")).unwrap();
        begin(&connection, "old", "token", "model", "openai.chat").unwrap();
        record(
            &connection,
            "old",
            Some((20, 5)),
            "succeeded",
            false,
            Some(30),
            Some(10),
        )
        .unwrap();
        connection
            .execute(
                "UPDATE panel_requests SET started_at=strftime('%Y-%m-%d','now','-100 days')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE panel_usage_daily SET day=strftime('%Y-%m-%d','now','-100 days')",
                [],
            )
            .unwrap();
        cleanup(&mut connection).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM panel_requests", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT SUM(input_tokens) FROM panel_usage_daily", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            20
        );
        connection
            .execute("UPDATE panel_migrations SET checksum='modified'", [])
            .unwrap();
        assert!(migrate(&mut connection).is_err());
    }
}
