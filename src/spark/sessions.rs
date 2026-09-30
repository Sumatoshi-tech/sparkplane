//! Private per-launch inference ledger; all SQL runs on the existing database actor.
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use secrecy::SecretString;
use tokio::sync::oneshot;

use super::{
    state::{DbActor, StateError},
    wire::{LaunchSessionCreated, LaunchSessionDocument, LaunchSessionRequest, SessionUsage},
};

pub(crate) const MIGRATION: &str = r#"
CREATE TABLE launch_sessions (
 id TEXT PRIMARY KEY, token_id TEXT NOT NULL UNIQUE REFERENCES token_metadata(id),
 request_hash TEXT NOT NULL, instance TEXT NOT NULL, model TEXT NOT NULL,
 integration TEXT NOT NULL, eco_mode TEXT NOT NULL, started_at TEXT NOT NULL,
 finished_at TEXT
);
CREATE TABLE launch_requests (
 id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES launch_sessions(id) ON DELETE CASCADE,
 instance TEXT NOT NULL, started_at TEXT NOT NULL, finished_at TEXT,
 input_tokens INTEGER, output_tokens INTEGER, outcome TEXT NOT NULL DEFAULT 'pending', accounting_error INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX launch_requests_session_idx ON launch_requests(session_id);
CREATE INDEX launch_sessions_retention_idx ON launch_sessions(started_at);
"#;

pub(crate) fn migration_needed(db: &Connection) -> Result<bool, StateError> {
    db.query_row("SELECT NOT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='launch_session_migrations')",[],|row|row.get(0)).map_err(sql)
}

pub(crate) fn migrate(db: &mut Connection) -> Result<(), StateError> {
    use sha2::{Digest, Sha256};
    let checksum = format!("{:x}", Sha256::digest(MIGRATION.as_bytes()));
    if !migration_needed(db)? {
        let versions = db
            .prepare("SELECT version,checksum FROM launch_session_migrations ORDER BY version")
            .map_err(sql)?
            .query_map([], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        if versions != [(1, checksum)] {
            return Err(StateError::Unavailable(
                "unsupported or modified session ledger migration".into(),
            ));
        }
        return Ok(());
    }
    let tx = db.transaction().map_err(sql)?;
    tx.execute_batch("CREATE TABLE launch_session_migrations(version INTEGER PRIMARY KEY,checksum TEXT NOT NULL);").map_err(sql)?;
    tx.execute_batch(MIGRATION).map_err(sql)?;
    tx.execute(
        "INSERT INTO launch_session_migrations(version,checksum) VALUES(1,?1)",
        [checksum],
    )
    .map_err(sql)?;
    tx.commit().map_err(sql)
}

pub(crate) enum Command {
    Create {
        actor: String,
        request: LaunchSessionRequest,
        reply: oneshot::Sender<Result<LaunchSessionCreated, StateError>>,
    },
    Get {
        id: String,
        finish: bool,
        actor: String,
        reply: oneshot::Sender<Result<LaunchSessionDocument, StateError>>,
    },
    Begin {
        token: String,
        id: String,
        instance: String,
        reply: oneshot::Sender<Result<bool, StateError>>,
    },
    Record {
        id: String,
        usage: Option<(u64, u64)>,
        outcome: String,
        incomplete: bool,
        reply: oneshot::Sender<Result<(), StateError>>,
    },
}

impl DbActor {
    pub async fn create_launch_session(
        &self,
        actor: &str,
        request: LaunchSessionRequest,
    ) -> Result<LaunchSessionCreated, StateError> {
        let (reply, receive) = oneshot::channel();
        self.session_command(Command::Create {
            actor: actor.into(),
            request,
            reply,
        })?;
        receive
            .await
            .map_err(|_| StateError::Unavailable("session database unavailable".into()))?
    }
    pub async fn launch_session(
        &self,
        id: &str,
        finish: bool,
        actor: &str,
    ) -> Result<LaunchSessionDocument, StateError> {
        let (reply, receive) = oneshot::channel();
        self.session_command(Command::Get {
            id: id.into(),
            finish,
            actor: actor.into(),
            reply,
        })?;
        receive
            .await
            .map_err(|_| StateError::Unavailable("session database unavailable".into()))?
    }
    async fn begin_usage(&self, token: &str, id: &str, instance: &str) -> Result<bool, StateError> {
        let (reply, receive) = oneshot::channel();
        self.session_command(Command::Begin {
            token: token.into(),
            id: id.into(),
            instance: instance.into(),
            reply,
        })?;
        receive
            .await
            .map_err(|_| StateError::Unavailable("session database unavailable".into()))?
    }
    async fn record_usage(
        &self,
        id: &str,
        usage: Option<(u64, u64)>,
        outcome: &str,
        incomplete: bool,
    ) -> Result<(), StateError> {
        let (reply, receive) = oneshot::channel();
        self.session_command(Command::Record {
            id: id.into(),
            usage,
            outcome: outcome.into(),
            incomplete,
            reply,
        })?;
        receive
            .await
            .map_err(|_| StateError::Unavailable("session database unavailable".into()))?
    }
}

fn sql(error: rusqlite::Error) -> StateError {
    StateError::Unavailable(format!("session database: {error}"))
}

pub(crate) fn dispatch(db: &mut Connection, pepper: &SecretString, command: Command) {
    match command {
        Command::Create {
            actor,
            request,
            reply,
        } => {
            let _ = reply.send(create(db, pepper, &actor, request));
        }
        Command::Get {
            id,
            finish,
            actor,
            reply,
        } => {
            let result = if finish {
                finish_session(db, &id, &actor).and_then(|()| read(db, &id))
            } else {
                read(db, &id)
            };
            let _ = reply.send(result);
        }
        Command::Begin {
            token,
            id,
            instance,
            reply,
        } => {
            let result = (|| {
                let expected = db
                    .query_row(
                        "SELECT instance,finished_at FROM launch_sessions WHERE token_id=?1",
                        [&token],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                    )
                    .optional()
                    .map_err(sql)?;
                let Some((expected, finished)) = expected else {
                    return Ok(false);
                };
                if expected != instance || finished.is_some() {
                    return Err(StateError::Invalid(
                        "launch session is not active for this instance".into(),
                    ));
                }
                db.execute("INSERT INTO launch_requests(id,session_id,instance,started_at) SELECT ?1,id,?2,?3 FROM launch_sessions WHERE token_id=?4 AND finished_at IS NULL", params![id,instance,super::state::now(),token]).map(|n| n == 1).map_err(sql)
            })();
            let _ = reply.send(result);
        }
        Command::Record {
            id,
            usage,
            outcome,
            incomplete,
            reply,
        } => {
            let result = usage.map(|(a,b)| (i64::try_from(a),i64::try_from(b))).map(|(a,b)| Ok((a.map_err(|_| StateError::Invalid("usage overflow".into()))?,b.map_err(|_| StateError::Invalid("usage overflow".into()))?))).transpose().and_then(|counts| {
                db.execute("UPDATE launch_requests SET input_tokens=?2,output_tokens=?3,outcome=?4,finished_at=?5,accounting_error=?6 WHERE id=?1 AND finished_at IS NULL",params![id,counts.map(|c|c.0),counts.map(|c|c.1),outcome,super::state::now(),incomplete]).map(|_|()).map_err(sql)
            });
            let _ = reply.send(result);
        }
    }
}

fn create(
    db: &mut Connection,
    pepper: &SecretString,
    actor: &str,
    request: LaunchSessionRequest,
) -> Result<LaunchSessionCreated, StateError> {
    if request.id.parse::<ulid::Ulid>().is_err()
        || !["codex", "claude", "opencode"].contains(&request.integration.as_str())
        || !["max", "none"].contains(&request.eco_mode.as_str())
    {
        return Err(StateError::Invalid("invalid launch session".into()));
    }
    let hash = super::wire::canonical_request_sha256(&request)
        .map_err(|_| StateError::Invalid("invalid session request".into()))?;
    if let Some(existing) = db
        .query_row(
            "SELECT request_hash FROM launch_sessions WHERE id=?1",
            [&request.id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(sql)?
    {
        if existing != hash {
            return Err(StateError::Conflict("session identity conflict".into()));
        }
        return Ok(LaunchSessionCreated {
            session: read(db, &request.id)?,
            bearer_token: None,
        });
    }
    let instance = super::state::read_instance(db, &request.instance)?;
    let active: i64 = db.query_row("SELECT COUNT(*) FROM token_metadata WHERE revoked_at IS NULL AND (expires_at IS NULL OR julianday(expires_at)>julianday('now'))",[],|r|r.get(0)).map_err(sql)?;
    if active >= 1024 {
        return Err(StateError::Overloaded);
    }
    let token_id = ulid::Ulid::new().to_string();
    let secret = super::state::random_secret()?;
    let verifier = super::state::token_hmac(pepper, &token_id, &secret)?;
    let expires = (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339();
    let now = super::state::now();
    let tx = db.transaction().map_err(sql)?;
    tx.execute(
        "DELETE FROM launch_sessions WHERE julianday(started_at)<julianday('now','-90 days')",
        [],
    )
    .map_err(sql)?;
    tx.execute("INSERT INTO token_metadata(id,name,verifier,scopes_json,allowed_cidrs_json,expires_at,max_concurrent_inference,created_at) VALUES(?1,?2,?3,'[\"inference\"]','[]',?4,8,?5)",params![token_id,format!("sparkplane-session@{}",request.id),verifier.as_slice(),expires,now]).map_err(sql)?;
    tx.execute("INSERT INTO launch_sessions(id,token_id,request_hash,instance,model,integration,eco_mode,started_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![request.id,token_id,hash,instance.name,instance.model,request.integration,request.eco_mode,now]).map_err(sql)?;
    tx.execute("INSERT INTO audit(occurred_at,actor_token_id,action,target,outcome,metadata_json) VALUES(?1,?2,'launch-session.create',?3,'succeeded','{}')",params![now,actor,request.id]).map_err(sql)?;
    tx.commit().map_err(sql)?;
    Ok(LaunchSessionCreated {
        session: read(db, &request.id)?,
        bearer_token: Some(format!("sparkplane_{token_id}_{secret}")),
    })
}

fn finish_session(db: &mut Connection, id: &str, actor: &str) -> Result<(), StateError> {
    let token: String = db
        .query_row(
            "SELECT token_id FROM launch_sessions WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .ok_or(StateError::NotFound)?;
    let tx = db.transaction().map_err(sql)?;
    let now = super::state::now();
    let changed = tx
        .execute(
            "UPDATE launch_sessions SET finished_at=?2 WHERE id=?1 AND finished_at IS NULL",
            params![id, now],
        )
        .map_err(sql)?;
    tx.execute(
        "UPDATE token_metadata SET revoked_at=COALESCE(revoked_at,?2) WHERE id=?1",
        params![token, now],
    )
    .map_err(sql)?;
    if changed > 0 {
        tx.execute("INSERT INTO audit(occurred_at,actor_token_id,action,target,outcome,metadata_json) VALUES(?1,?2,'launch-session.finish',?3,'succeeded','{}')",params![now,actor,id]).map_err(sql)?;
    }
    tx.commit().map_err(sql)
}

fn read(db: &Connection, id: &str) -> Result<LaunchSessionDocument, StateError> {
    let mut session = db.query_row("SELECT id,instance,model,integration,eco_mode,started_at,finished_at FROM launch_sessions WHERE id=?1",[id],|r|Ok(LaunchSessionDocument { schema:"sparkplane.launch-session/v1".into(),id:r.get(0)?,instance:r.get(1)?,model:r.get(2)?,integration:r.get(3)?,eco_mode:r.get(4)?,started_at:r.get(5)?,finished_at:r.get(6)?,usage:SessionUsage::default() })).optional().map_err(sql)?.ok_or(StateError::NotFound)?;
    session.usage = db.query_row("SELECT COUNT(*),COALESCE(SUM(input_tokens),0),COALESCE(SUM(output_tokens),0),COALESCE(SUM(input_tokens IS NULL),0),COALESCE(SUM(finished_at IS NULL),0),COALESCE(SUM(outcome!='succeeded' AND outcome!='pending'),0),COALESCE(SUM(CASE WHEN input_tokens>272000 THEN input_tokens ELSE 0 END),0),COALESCE(SUM(CASE WHEN input_tokens>272000 THEN output_tokens ELSE 0 END),0),COALESCE(SUM(accounting_error),0) FROM launch_requests WHERE session_id=?1",[id],|r|Ok(SessionUsage {requests:r.get::<_,i64>(0)? as u64,input_tokens:r.get::<_,i64>(1)? as u64,output_tokens:r.get::<_,i64>(2)? as u64,unknown_usage_requests:r.get::<_,i64>(3)? as u64,pending_requests:r.get::<_,i64>(4)? as u64,failed_requests:r.get::<_,i64>(5)? as u64,long_context_input_tokens:r.get::<_,i64>(6)? as u64,long_context_output_tokens:r.get::<_,i64>(7)? as u64,accounting_errors:r.get::<_,i64>(8)? as u64})).map_err(sql)?;
    Ok(session)
}

#[derive(Clone, Default)]
pub(crate) struct UsageMeter(Option<Arc<MeterInner>>);
impl std::fmt::Debug for UsageMeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UsageMeter")
    }
}
#[derive(Clone, Copy)]
struct MeterValues {
    usage: Option<(u64, u64)>,
    outcome: &'static str,
    incomplete: bool,
}
struct MeterInner {
    db: DbActor,
    id: String,
    values: Mutex<MeterValues>,
}
impl UsageMeter {
    pub async fn begin(
        db: Option<&DbActor>,
        token: &str,
        instance: &str,
    ) -> Result<Self, StateError> {
        let Some(db) = db else {
            return Ok(Self::default());
        };
        let id = ulid::Ulid::new().to_string();
        match db.begin_usage(token, &id, instance).await {
            Ok(true) => Ok(Self(Some(Arc::new(MeterInner {
                db: db.clone(),
                id,
                values: Mutex::new(MeterValues {
                    usage: None,
                    outcome: "interrupted",
                    incomplete: false,
                }),
            })))),
            Ok(false) => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }
    pub fn observe(&self, event: &super::upstream::GenerationEvent) {
        if let Some(inner) = &self.0
            && let Ok(mut values) = inner.values.lock()
        {
            match event {
                super::upstream::GenerationEvent::Usage {
                    prompt_tokens,
                    completion_tokens,
                } => values.usage = Some((*prompt_tokens, *completion_tokens)),
                super::upstream::GenerationEvent::Done => values.outcome = "succeeded",
                _ => (),
            }
        }
    }
    pub fn json(&self, bytes: &[u8], success: bool) {
        if let Some(inner) = &self.0
            && let Ok(mut values) = inner.values.lock()
        {
            values.outcome = if success { "succeeded" } else { "failed" };
            if let Ok(document) = serde_json::from_slice::<serde_json::Value>(bytes)
                && let Some(usage) = document.get("usage")
            {
                values.usage = usage
                    .get("prompt_tokens")
                    .or_else(|| usage.get("input_tokens"))
                    .and_then(|v| v.as_u64())
                    .zip(
                        usage
                            .get("completion_tokens")
                            .or_else(|| usage.get("output_tokens"))
                            .and_then(|v| v.as_u64()),
                    );
            }
        }
    }
    pub fn raw(&self, bytes: &[u8]) {
        // RawResponseStream frames events; SSE permits data: with no space and
        // JSON spread across several data lines, with LF or CRLF separators.
        let data = bytes
            .split(|b| *b == b'\n')
            .filter_map(|line| {
                line.strip_suffix(b"\r")
                    .unwrap_or(line)
                    .strip_prefix(b"data:")
            })
            .map(|line| line.strip_prefix(b" ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join(&b'\n');
        if let Ok(document) = serde_json::from_slice::<serde_json::Value>(&data) {
            let response = document.get("response").unwrap_or(&document);
            let terminal = matches!(
                document.get("type").and_then(|v| v.as_str()),
                Some("response.completed" | "response.incomplete" | "response.failed")
            );
            let success =
                document.get("type").and_then(|v| v.as_str()) == Some("response.completed");
            if terminal || response.get("usage").is_some() {
                self.json(&serde_json::to_vec(response).unwrap_or_default(), success);
            }
        }
    }
}
impl Drop for MeterInner {
    fn drop(&mut self) {
        let mut values = self.values.lock().map(|v| *v).unwrap_or(MeterValues {
            usage: None,
            outcome: "interrupted",
            incomplete: true,
        });
        values.incomplete |= values.outcome != "succeeded";
        let db = self.db.clone();
        let id = self.id.clone();
        tokio::spawn(async move {
            if db
                .record_usage(&id, values.usage, values.outcome, values.incomplete)
                .await
                .is_err()
            {
                tracing::warn!("session usage could not be recorded");
            }
        });
    }
}

impl super::upstream::UsageObserver for UsageMeter {
    fn observe(&self, event: &super::upstream::GenerationEvent) {
        UsageMeter::observe(self, event);
    }
    fn json(&self, bytes: &[u8], success: bool) {
        UsageMeter::json(self, bytes, success);
    }
    fn raw(&self, bytes: &[u8]) {
        UsageMeter::raw(self, bytes);
    }
    fn incomplete(&self) {
        if let Some(inner) = &self.0
            && let Ok(mut values) = inner.values.lock()
        {
            values.incomplete = true;
        }
    }
}
