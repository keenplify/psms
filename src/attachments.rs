//! Photos and other files that came with a text. Each one is sealed with the archive key like
//! the message itself, so this service stores envelopes only. They are uploaded after their
//! message, numbered from 0, and deleted with it.
use crate::{Archive, encryption};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS message_attachments (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL CHECK (idx BETWEEN 0 AND 9),
    envelope TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (message_id, idx)
)";
/// The largest sealed attachment, in base64 characters (about 1.5 MB of file).
pub const MAX_ENVELOPE: usize = 2 * 1024 * 1024;
/// Attachments per message.
pub const MAX_COUNT: u8 = 10;

#[derive(Deserialize)]
struct Upload {
    envelope: String,
}

/// The message ID as the archive stores it, when the path names a real attachment slot.
fn slot(message: &str, index: u8) -> Option<String> {
    (index < MAX_COUNT)
        .then(|| uuid::Uuid::parse_str(message).ok())
        .flatten()
        .map(|id| id.to_string())
}

async fn put_one(
    State(archive): State<Archive>,
    headers: HeaderMap,
    Path((message, index)): Path<(String, u8)>,
    Json(upload): Json<Upload>,
) -> StatusCode {
    let phone = headers
        .get("authorization")
        .is_some_and(|v| bool::from(archive.credential.as_bytes().ct_eq(v.as_bytes())));
    if !phone && !archive.is_owner(&headers) {
        return StatusCode::UNAUTHORIZED;
    }
    if archive.remote.is_some() {
        return StatusCode::NOT_IMPLEMENTED;
    }
    let Some(message) = slot(&message, index).filter(|_| {
        upload.envelope.len() <= MAX_ENVELOPE
            && encryption::accepts(encryption::ACTIONS, &upload.envelope)
    }) else {
        return StatusCode::UNPROCESSABLE_ENTITY;
    };
    let arrivals = archive.arrivals.clone();
    let stored = tokio::task::spawn_blocking(move || {
        let db = archive.db.lock().map_err(|_| ())?;
        let exists = db
            .query_row("SELECT 1 FROM messages WHERE id=?1", [&message], |_| Ok(()))
            .optional()
            .map_err(|_| ())?
            .is_some();
        if !exists {
            return Ok(false);
        }
        db.execute(
            "INSERT INTO message_attachments(message_id,idx,envelope) VALUES(?1,?2,?3)
             ON CONFLICT(message_id,idx) DO UPDATE SET envelope=excluded.envelope",
            params![message, index, upload.envelope],
        )
        .map(|_| true)
        .map_err(|_| ())
    })
    .await;
    match stored {
        Ok(Ok(true)) => {
            arrivals.send_replace(());
            StatusCode::NO_CONTENT
        }
        Ok(Ok(false)) => StatusCode::NOT_FOUND,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn get_one(
    State(archive): State<Archive>,
    headers: HeaderMap,
    Path((message, index)): Path<(String, u8)>,
) -> Result<Json<Value>, StatusCode> {
    if !archive.is_owner(&headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if archive.remote.is_some() {
        return Err(StatusCode::NOT_FOUND);
    }
    let message = slot(&message, index).ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
    tokio::task::spawn_blocking(move || {
        let db = archive
            .db
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        db.query_row(
            "SELECT envelope FROM message_attachments WHERE message_id=?1 AND idx=?2",
            params![message, index],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .map(|envelope| Json(json!({ "envelope": envelope })))
        .ok_or(StatusCode::NOT_FOUND)
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}

pub fn routes(prefix: &str, archive: Archive) -> Router<Archive> {
    Router::new()
        .route(
            &format!("{prefix}/messages/{{id}}/attachments/{{index}}"),
            get(get_one)
                .put(put_one)
                .layer(DefaultBodyLimit::max(MAX_ENVELOPE + 4096)),
        )
        .with_state(archive)
}
