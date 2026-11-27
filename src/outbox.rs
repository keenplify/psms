//! Messages queued for sending from any surface (web, CLI, MCP, other apps) and delivered by a
//! sending device: the Mac app through Messages today, an Android phone later. The service never
//! sends anything itself and keeps the text only until a device reports the message sent.
use crate::Archive;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS outbox(
    id TEXT PRIMARY KEY NOT NULL, recipient TEXT NOT NULL, body TEXT,
    status TEXT NOT NULL DEFAULT 'queued' CHECK(status IN ('queued','sending','sent','failed')),
    error TEXT, created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    claimed_at INTEGER, finished_at INTEGER)";
/// Scheduled texts: older archives gain the column in place.
pub fn migrate(db: &rusqlite::Connection) -> rusqlite::Result<()> {
    let has: bool = db
        .prepare("SELECT 1 FROM pragma_table_info('outbox') WHERE name='send_at'")?
        .exists([])?;
    if !has {
        db.execute_batch("ALTER TABLE outbox ADD COLUMN send_at INTEGER")?;
    }
    Ok(())
}
/// How far ahead a text can be scheduled.
const MAX_SCHEDULE: i64 = 366 * 24 * 3600;
/// A claimed message goes back to the queue if its device never reports a result.
const CLAIM_TIMEOUT: i64 = 300;

type Reply = Result<(StatusCode, Json<Value>), StatusCode>;

fn can_send(archive: &Archive, headers: &HeaderMap) -> bool {
    archive.is_owner(headers) || archive.app_token(headers, "send")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Queue {
    to: String,
    body: String,
    /// Send at this Unix time instead of now, like BlueBubbles' scheduled messages.
    #[serde(default)]
    send_at: Option<i64>,
}
async fn queue(State(archive): State<Archive>, headers: HeaderMap, Json(q): Json<Queue>) -> Reply {
    if !can_send(&archive, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let to = q.to.trim().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    if to.is_empty()
        || to.len() > 64
        || q.body.trim().is_empty()
        || q.body.len() > 4000
        || q.send_at.is_some_and(|t| t > now + MAX_SCHEDULE)
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let send_at = q.send_at.filter(|t| *t > now);
    let id = Uuid::new_v4().to_string();
    let saved = id.clone();
    archive
        .query(move |db| {
            db.execute(
                "INSERT INTO outbox(id,recipient,body,send_at) VALUES(?1,?2,?3,?4)",
                params![saved, to, q.body, send_at],
            )
        })
        .await?;
    archive.arrivals.send_replace(());
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"id": id, "status": "queued", "send_at": send_at})),
    ))
}

async fn status(State(archive): State<Archive>, headers: HeaderMap, Path(id): Path<Uuid>) -> Reply {
    if !can_send(&archive, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let found = archive
        .query(move |db| {
            db.query_row(
                "SELECT status,error,created_at,finished_at,send_at FROM outbox WHERE id=?1",
                [id.to_string()],
                |r| {
                    Ok(json!({"id": id, "status": r.get::<_, String>(0)?, "error": r.get::<_, Option<String>>(1)?,
                        "created_at": r.get::<_, i64>(2)?, "finished_at": r.get::<_, Option<i64>>(3)?,
                        "send_at": r.get::<_, Option<i64>>(4)?}))
                },
            )
            .optional()
        })
        .await?;
    found
        .map(|v| (StatusCode::OK, Json(v)))
        .ok_or(StatusCode::NOT_FOUND)
}

/// A sending device takes the next queued messages. Only the archive owner can claim.
async fn claim(State(archive): State<Archive>, headers: HeaderMap) -> Reply {
    if !archive.is_owner(&headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let items = archive
        .query(|db| {
            db.execute(
                "UPDATE outbox SET status='queued', claimed_at=NULL WHERE status='sending' AND claimed_at < unixepoch() - ?1",
                [CLAIM_TIMEOUT],
            )?;
            let mut stmt = db.prepare(
                "UPDATE outbox SET status='sending', claimed_at=unixepoch()
                 WHERE id IN (SELECT id FROM outbox WHERE status='queued' AND (send_at IS NULL OR send_at <= unixepoch())
                              ORDER BY coalesce(send_at, created_at) LIMIT 10)
                 RETURNING id, recipient, body",
            )?;
            stmt.query_map([], |r| {
                Ok(json!({"id": r.get::<_, String>(0)?, "to": r.get::<_, String>(1)?, "body": r.get::<_, String>(2)?}))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .and_then(|items| {
                let newest: Option<i64> =
                    db.query_row("SELECT max(archived_at) FROM messages", [], |r| r.get(0))?;
                Ok((items, newest))
            })
        })
        .await?;
    let (items, newest) = items;
    Ok((
        StatusCode::OK,
        Json(json!({"messages": items, "newest": newest})),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Outcome {
    status: String,
    error: Option<String>,
}
async fn result(
    State(archive): State<Archive>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(outcome): Json<Outcome>,
) -> Result<StatusCode, StatusCode> {
    if !archive.is_owner(&headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let sent = match outcome.status.as_str() {
        "sent" => true,
        "failed" => false,
        _ => return Err(StatusCode::UNPROCESSABLE_ENTITY),
    };
    let error = outcome
        .error
        .map(|e| e.chars().take(300).collect::<String>());
    let changed = archive
        .query(move |db| {
            // The text is no longer needed once delivered; the sent copy lives in the archive.
            db.execute(
                "UPDATE outbox SET status=?2, error=?3, finished_at=unixepoch(),
                    body=CASE WHEN ?2='sent' THEN NULL ELSE body END
                 WHERE id=?1 AND status='sending'",
                params![id.to_string(), if sent { "sent" } else { "failed" }, error],
            )
        })
        .await?;
    if changed == 0 {
        Err(StatusCode::CONFLICT)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

/// Runs the device API while it is turned on, following the setting without a restart.
pub fn watch_devices(archive: Archive, prefix: String) {
    tokio::spawn(async move {
        let mut running: Option<(u16, tokio::task::JoinHandle<()>)> = None;
        let mut failed = None;
        loop {
            let wanted = archive.query(crate::device_api::port).await.ok().flatten();
            if running.as_ref().map(|r| r.0) != wanted {
                if let Some((_, task)) = running.take() {
                    task.abort();
                }
                if let Some(port) = wanted.filter(|p| failed != Some(*p)) {
                    match tokio::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port))
                        .await
                    {
                        Ok(listener) => {
                            eprintln!("Device API listening on port {port}");
                            failed = None;
                            let app = device_routes(&prefix, archive.clone());
                            running = Some((
                                port,
                                tokio::spawn(async move {
                                    let _ = axum::serve(
                                        listener,
                                        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                                    )
                                    .await;
                                }),
                            ));
                        }
                        Err(_) => {
                            eprintln!("Device API could not use port {port}");
                            failed = Some(port);
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });
}
/// Private networks only, and only a send-scope key: this listener can queue texts and
/// check on them, and nothing else.
async fn devices_only(
    State(archive): State<Archive>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if !crate::device_api::private(peer.ip()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !archive.app_token(request.headers(), "send") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}
fn device_routes(prefix: &str, archive: Archive) -> Router {
    Router::new()
        .route(&format!("{prefix}/outbox"), post(queue))
        .route(&format!("{prefix}/outbox/{{id}}"), get(status))
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024))
        .layer(axum::middleware::from_fn_with_state(
            archive.clone(),
            devices_only,
        ))
        .with_state(archive)
}

pub fn routes(prefix: &str, archive: Archive) -> Router<Archive> {
    Router::new()
        .route(&format!("{prefix}/outbox"), post(queue))
        .route(&format!("{prefix}/outbox/claim"), post(claim))
        .route(&format!("{prefix}/outbox/{{id}}"), get(status))
        .route(&format!("{prefix}/outbox/{{id}}/result"), post(result))
        .with_state(archive)
}
