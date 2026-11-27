//! Contact cards for the local archive. Cards are sealed with the archive key on the phone or
//! in an inbox, so this service only stores envelopes and timestamps. The phone's copy comes
//! from its address book; the manual copy holds edits and wins when both exist.
use crate::{Archive, encryption};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS contact_cards(
    contact_id TEXT PRIMARY KEY NOT NULL CHECK(length(contact_id)=64),
    phone_card TEXT, phone_updated_at INTEGER,
    manual_card TEXT, manual_updated_at INTEGER)";
/// Phones refresh a card at most this often.
const STALE_AFTER: i64 = 30 * 86_400;

/// Tells the phone whether to send a card for this sender, from timestamps alone.
pub fn hint(db: &Connection, contact: Option<&str>) -> Value {
    let Some(contact) = contact else {
        return json!({});
    };
    let fresh = db
        .query_row(
            "SELECT phone_updated_at > unixepoch() - ?2 FROM contact_cards WHERE contact_id=?1",
            params![contact, STALE_AFTER],
            |r| r.get::<_, Option<bool>>(0),
        )
        .optional()
        .ok()
        .flatten()
        .flatten()
        .unwrap_or(false);
    if fresh {
        json!({})
    } else {
        json!({"contact_card": "wanted"})
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Card {
    envelope: Option<String>,
}

fn valid_contact(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

async fn save(
    State(archive): State<Archive>,
    headers: HeaderMap,
    Path(contact): Path<String>,
    Json(card): Json<Card>,
) -> StatusCode {
    let phone = headers
        .get("authorization")
        .is_some_and(|v| bool::from(archive.credential.as_bytes().ct_eq(v.as_bytes())));
    let owner = archive.is_owner(&headers);
    if !phone && !owner {
        return StatusCode::UNAUTHORIZED;
    }
    if archive.remote.is_some() {
        return StatusCode::NOT_IMPLEMENTED;
    }
    if !valid_contact(&contact)
        || card
            .envelope
            .as_ref()
            .is_some_and(|e| e.len() > 400_000 || !encryption::accepts(encryption::ACTIONS, e))
    {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    let sql = match (phone, card.envelope.is_some()) {
        (true, true) => {
            "INSERT INTO contact_cards(contact_id,phone_card,phone_updated_at) VALUES(?1,?2,unixepoch())
             ON CONFLICT(contact_id) DO UPDATE SET phone_card=excluded.phone_card, phone_updated_at=unixepoch()"
        }
        (false, true) => {
            "INSERT INTO contact_cards(contact_id,manual_card,manual_updated_at) VALUES(?1,?2,unixepoch())
             ON CONFLICT(contact_id) DO UPDATE SET manual_card=excluded.manual_card, manual_updated_at=unixepoch()"
        }
        (false, false) => {
            "UPDATE contact_cards SET manual_card=NULL, manual_updated_at=unixepoch() WHERE contact_id=?1 AND ?2 IS NULL"
        }
        (true, false) => return StatusCode::UNPROCESSABLE_ENTITY,
    };
    let arrivals = archive.arrivals.clone();
    let stored = tokio::task::spawn_blocking(move || {
        let db = archive.db.lock().map_err(|_| ())?;
        db.execute(sql, params![contact, card.envelope])
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

async fn list(
    State(archive): State<Archive>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    if !archive.is_owner(&headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if archive.remote.is_some() {
        return Ok(Json(json!({"cards": []})));
    }
    tokio::task::spawn_blocking(move || {
        let db = archive.db.lock().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let mut stmt = db
            .prepare("SELECT contact_id,phone_card,phone_updated_at,manual_card,manual_updated_at FROM contact_cards WHERE phone_card IS NOT NULL OR manual_card IS NOT NULL")
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let cards = stmt
            .query_map([], |r| {
                Ok(json!({
                    "contact_id": r.get::<_, String>(0)?,
                    "phone_card": r.get::<_, Option<String>>(1)?,
                    "phone_updated_at": r.get::<_, Option<i64>>(2)?,
                    "manual_card": r.get::<_, Option<String>>(3)?,
                    "manual_updated_at": r.get::<_, Option<i64>>(4)?,
                }))
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        Ok(Json(json!({ "cards": cards })))
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}

pub fn routes(prefix: &str, archive: Archive) -> Router<Archive> {
    Router::new()
        .route(&format!("{prefix}/contacts/cards"), get(list))
        .route(
            &format!("{prefix}/contacts/{{id}}/card"),
            put(save).layer(DefaultBodyLimit::max(512 * 1024)),
        )
        .with_state(archive)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asks_for_a_card_until_the_phone_sends_a_fresh_one() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(SCHEMA).unwrap();
        let id = "a".repeat(64);
        assert_eq!(hint(&db, None), json!({}));
        assert_eq!(hint(&db, Some(&id)), json!({"contact_card": "wanted"}));
        db.execute(
            "INSERT INTO contact_cards(contact_id,manual_card,manual_updated_at) VALUES(?1,'x',unixepoch())",
            [&id],
        )
        .unwrap();
        assert_eq!(hint(&db, Some(&id)), json!({"contact_card": "wanted"}));
        db.execute(
            "UPDATE contact_cards SET phone_card='y', phone_updated_at=unixepoch() WHERE contact_id=?1",
            [&id],
        )
        .unwrap();
        assert_eq!(hint(&db, Some(&id)), json!({}));
        db.execute(
            "UPDATE contact_cards SET phone_updated_at=unixepoch()-31*86400 WHERE contact_id=?1",
            [&id],
        )
        .unwrap();
        assert_eq!(hint(&db, Some(&id)), json!({"contact_card": "wanted"}));
        assert!(valid_contact(&id) && !valid_contact(&"A".repeat(64)));
    }
}
