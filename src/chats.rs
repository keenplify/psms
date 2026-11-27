//! Conversations and reactions, stored end-to-end encrypted. A chat's name, members and the
//! Messages identifier pSMS for Mac sends to are sealed in its envelope; the server only sees
//! an opaque 32-character ID that its messages and reactions share. A reaction is sealed too
//! (who, which tapback) and hangs off the message it reacts to, so it never counts as a text.
use crate::{Archive, encryption};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS chats (
    id TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 32),
    envelope TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE IF NOT EXISTS message_chats (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    chat_id TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS message_chats_chat ON message_chats(chat_id);
CREATE TABLE IF NOT EXISTS message_reactions (
    id TEXT PRIMARY KEY NOT NULL,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    envelope TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX IF NOT EXISTS message_reactions_message ON message_reactions(message_id);
";
const MAX_CHAT: usize = 64 * 1024;
const MAX_REACTION: usize = 8 * 1024;
const REACTIONS_PER_MESSAGE: i64 = 100;

/// A chat ID: 32 lowercase hex characters, opaque to the server.
pub fn valid_chat(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn uuid(id: &str) -> Option<String> {
    uuid::Uuid::parse_str(id).ok().map(|u| u.to_string())
}

#[derive(Deserialize)]
struct Sealed {
    envelope: String,
}

fn writer(archive: &Archive, headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .is_some_and(|v| bool::from(archive.credential.as_bytes().ct_eq(v.as_bytes())))
        || archive.is_owner(headers)
}

async fn put_chat(
    State(archive): State<Archive>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(chat): Json<Sealed>,
) -> StatusCode {
    if !writer(&archive, &headers) {
        return StatusCode::UNAUTHORIZED;
    }
    if archive.remote.is_some() {
        return StatusCode::NOT_IMPLEMENTED;
    }
    if !valid_chat(&id)
        || chat.envelope.len() > MAX_CHAT
        || !encryption::accepts(encryption::ACTIONS, &chat.envelope)
    {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    let arrivals = archive.arrivals.clone();
    let stored = tokio::task::spawn_blocking(move || {
        archive
            .db
            .lock()
            .map_err(|_| ())?
            .execute(
                "INSERT INTO chats(id,envelope) VALUES(?1,?2)
             ON CONFLICT(id) DO UPDATE SET envelope=excluded.envelope, updated_at=unixepoch()",
                params![id, chat.envelope],
            )
            .map_err(|_| ())
    })
    .await;
    match stored {
        Ok(Ok(_)) => {
            arrivals.send_replace(());
            StatusCode::NO_CONTENT
        }
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn list_chats(
    State(archive): State<Archive>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    if !archive.is_owner(&headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if archive.remote.is_some() {
        return Ok(Json(json!({"chats": []})));
    }
    tokio::task::spawn_blocking(move || {
        let db = archive.db.lock().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let mut stmt = db
            .prepare("SELECT id,envelope,updated_at FROM chats ORDER BY updated_at DESC")
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let chats = stmt
            .query_map([], |r| {
                Ok(json!({"id": r.get::<_, String>(0)?, "envelope": r.get::<_, String>(1)?, "updated_at": r.get::<_, i64>(2)?}))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        Ok(Json(json!({ "chats": chats })))
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}

async fn put_reaction(
    State(archive): State<Archive>,
    headers: HeaderMap,
    Path((message, reaction)): Path<(String, String)>,
    Json(sealed): Json<Sealed>,
) -> StatusCode {
    if !writer(&archive, &headers) {
        return StatusCode::UNAUTHORIZED;
    }
    if archive.remote.is_some() {
        return StatusCode::NOT_IMPLEMENTED;
    }
    let (Some(message), Some(reaction)) = (uuid(&message), uuid(&reaction)) else {
        return StatusCode::UNPROCESSABLE_ENTITY;
    };
    if sealed.envelope.len() > MAX_REACTION
        || !encryption::accepts(encryption::ACTIONS, &sealed.envelope)
    {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    let arrivals = archive.arrivals.clone();
    let stored = tokio::task::spawn_blocking(move || {
        let db = archive.db.lock().map_err(|_| ())?;
        let count: Option<i64> = db
            .query_row(
                "SELECT (SELECT count(*) FROM message_reactions WHERE message_id=?1) FROM messages WHERE id=?1",
                [&message],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| ())?;
        match count {
            None => Ok(StatusCode::NOT_FOUND),
            Some(n) if n >= REACTIONS_PER_MESSAGE => Ok(StatusCode::PAYLOAD_TOO_LARGE),
            Some(_) => db
                .execute(
                    "INSERT INTO message_reactions(id,message_id,envelope) VALUES(?1,?2,?3)
                     ON CONFLICT(id) DO UPDATE SET envelope=excluded.envelope
                     WHERE message_reactions.message_id=excluded.message_id",
                    params![reaction, message, sealed.envelope],
                )
                .map(|n| if n == 1 { StatusCode::NO_CONTENT } else { StatusCode::CONFLICT })
                .map_err(|_| ()),
        }
    })
    .await;
    match stored {
        Ok(Ok(StatusCode::NO_CONTENT)) => {
            arrivals.send_replace(());
            StatusCode::NO_CONTENT
        }
        Ok(Ok(status)) => status,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn delete_reaction(
    State(archive): State<Archive>,
    headers: HeaderMap,
    Path((message, reaction)): Path<(String, String)>,
) -> StatusCode {
    if !writer(&archive, &headers) {
        return StatusCode::UNAUTHORIZED;
    }
    if archive.remote.is_some() {
        return StatusCode::NOT_IMPLEMENTED;
    }
    let (Some(message), Some(reaction)) = (uuid(&message), uuid(&reaction)) else {
        return StatusCode::UNPROCESSABLE_ENTITY;
    };
    let arrivals = archive.arrivals.clone();
    let removed = tokio::task::spawn_blocking(move || {
        archive
            .db
            .lock()
            .map_err(|_| ())?
            .execute(
                "DELETE FROM message_reactions WHERE id=?1 AND message_id=?2",
                params![reaction, message],
            )
            .map_err(|_| ())
    })
    .await;
    match removed {
        Ok(Ok(1)) => {
            arrivals.send_replace(());
            StatusCode::NO_CONTENT
        }
        Ok(Ok(_)) => StatusCode::NOT_FOUND,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

pub fn routes(prefix: &str, archive: Archive) -> Router<Archive> {
    Router::new()
        .route(&format!("{prefix}/chats"), get(list_chats))
        .route(
            &format!("{prefix}/chats/{{id}}"),
            put(put_chat).layer(DefaultBodyLimit::max(MAX_CHAT + 1024)),
        )
        .route(
            &format!("{prefix}/messages/{{id}}/reactions/{{reaction}}"),
            put(put_reaction).delete(delete_reaction),
        )
        .with_state(archive)
}
