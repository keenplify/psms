//! Is the phone still sending? iOS sometimes turns automations off without saying so, so the
//! archive keeps three signals clients can warn on: when the phone last sent a text, when it
//! last checked in (a daily Time of Day automation running pSMS Check-in), and when pSMS for Mac
//! last saw a text in Messages that the phone never sent.
use crate::Archive;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS capture_health(
    id INTEGER PRIMARY KEY CHECK(id=1), last_upload_at INTEGER, checked_in_at INTEGER, missed_at INTEGER)";

fn phone(archive: &Archive, headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .is_some_and(|v| bool::from(archive.credential.as_bytes().ct_eq(v.as_bytes())))
}

/// Records that the phone sent a text. Called after an upload with the phone's token.
pub fn uploaded(db: &rusqlite::Connection) {
    let _ = db.execute(
        "INSERT INTO capture_health(id,last_upload_at) VALUES(1,unixepoch())
         ON CONFLICT(id) DO UPDATE SET last_upload_at=unixepoch()",
        [],
    );
}

async fn mark(archive: Archive, column: &'static str) -> StatusCode {
    let done = tokio::task::spawn_blocking(move || {
        archive.db.lock().map_err(|_| ())?.execute(
            &format!("INSERT INTO capture_health(id,{column}) VALUES(1,unixepoch()) ON CONFLICT(id) DO UPDATE SET {column}=unixepoch()"),
            [],
        ).map_err(|_| ())
    })
    .await;
    match done {
        Ok(Ok(_)) => StatusCode::NO_CONTENT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// The phone's daily check-in.
async fn checkin(State(archive): State<Archive>, headers: HeaderMap) -> StatusCode {
    if !phone(&archive, &headers) {
        return StatusCode::UNAUTHORIZED;
    }
    mark(archive, "checked_in_at").await
}

/// pSMS for Mac saw a text in Messages that the phone never sent.
async fn missed(State(archive): State<Archive>, headers: HeaderMap) -> StatusCode {
    if !archive.is_owner(&headers) {
        return StatusCode::UNAUTHORIZED;
    }
    mark(archive, "missed_at").await
}

async fn health(
    State(archive): State<Archive>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    if !archive.is_owner(&headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    tokio::task::spawn_blocking(move || {
        let db = archive
            .db
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let (last, checked, missed): (Option<i64>, Option<i64>, Option<i64>) = db
            .query_row(
                "SELECT last_upload_at, checked_in_at, missed_at FROM capture_health WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap_or((None, None, None));
        let recent: i64 = db
            .query_row(
                "SELECT count(*) FROM messages WHERE archived_at > unixepoch() - 7*86400",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok(Json(json!({
            "phones": [{"id": "phone", "last_message_at": last, "checked_in_at": checked}],
            "recent_messages_7d": recent,
            "missed_at": missed,
        })))
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}

pub fn routes(prefix: &str, archive: Archive) -> Router<Archive> {
    Router::new()
        .route(&format!("{prefix}/checkin"), post(checkin))
        .route(&format!("{prefix}/health"), get(health))
        .route(&format!("{prefix}/health/missed"), post(missed))
        .with_state(archive)
}
