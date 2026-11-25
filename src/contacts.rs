use crate::{
    Archive,
    inbox::{Entry, SELECT, entry},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Filters {
    contact_id: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
    include: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Contact {
    id: String,
    first_seen_at: i64,
    last_seen_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    messages: Option<MessagePage>,
}
#[derive(Serialize, Deserialize)]
struct ContactPage {
    contacts: Vec<Contact>,
    next_cursor: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct MessagePage {
    messages: Vec<Entry>,
    next_cursor: Option<String>,
}
type Result<T> = std::result::Result<T, StatusCode>;

fn digest(id: &str) -> Result<String> {
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(id.to_ascii_lowercase())
}
fn limit(n: Option<usize>) -> Result<usize> {
    let n = n.unwrap_or(30);
    if (1..=100).contains(&n) {
        Ok(n)
    } else {
        Err(StatusCode::BAD_REQUEST)
    }
}
fn expand(include: Option<&str>) -> Result<bool> {
    match include {
        None => Ok(false),
        Some("messages") => Ok(true),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}
fn contact(row: &rusqlite::Row) -> rusqlite::Result<Contact> {
    Ok(Contact {
        id: row.get(0)?,
        first_seen_at: row.get(1)?,
        last_seen_at: row.get(2)?,
        messages: None,
    })
}
fn messages(db: &Connection, id: &str, cursor: &str, n: usize) -> rusqlite::Result<MessagePage> {
    let mut stmt=db.prepare_cached(&format!("{SELECT} JOIN message_contacts c ON c.message_id=m.id WHERE c.contact_id=?1 AND m.id>?2 ORDER BY m.id LIMIT ?3"))?;
    let mut entries = stmt
        .query_map(params![id, cursor, n as i64 + 1], |r| {
            Ok((r.get::<_, String>(0)?, entry(r)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = entries.len() > n;
    entries.truncate(n);
    let next_cursor = more.then(|| entries.last().unwrap().0.clone());
    Ok(MessagePage {
        messages: entries.into_iter().map(|(_, m)| m).collect(),
        next_cursor,
    })
}
async fn list(
    State(archive): State<Archive>,
    Query(q): Query<Filters>,
) -> Result<Json<ContactPage>> {
    let n = limit(q.limit)?;
    let include = expand(q.include.as_deref())?;
    let cursor = q
        .cursor
        .as_deref()
        .map(digest)
        .transpose()?
        .unwrap_or_default();
    let filter = q.contact_id.as_deref().map(digest).transpose()?;
    if let Some(remote) = &archive.remote {
        return crate::inbox::remote_result(
            remote
                .contacts(filter.as_deref(), &cursor, n, include)
                .await,
        );
    }
    archive.query(move|db|{
        let sql=if filter.is_some(){"SELECT id,first_seen_at,last_seen_at FROM contacts WHERE id>?1 AND id=?3 ORDER BY id LIMIT ?2"}else{"SELECT id,first_seen_at,last_seen_at FROM contacts WHERE id>?1 ORDER BY id LIMIT ?2"};
        let mut stmt=db.prepare_cached(sql)?;
        let mut contacts=if let Some(filter)=filter {stmt.query_map(params![cursor,n as i64+1,filter],contact)?}else{stmt.query_map(params![cursor,n as i64+1],contact)?}.collect::<rusqlite::Result<Vec<_>>>()?;
        let more=contacts.len()>n;contacts.truncate(n);
        let next_cursor=more.then(||contacts.last().unwrap().id.clone());
        if include{for c in &mut contacts{c.messages=Some(messages(db,&c.id,"",5)?)}}
        Ok(ContactPage{contacts,next_cursor})
    }).await.map(Json)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Detail {
    include: Option<String>,
}
async fn detail(
    State(a): State<Archive>,
    Path(id): Path<String>,
    Query(q): Query<Detail>,
) -> Result<Json<Contact>> {
    let id = digest(&id)?;
    let include = expand(q.include.as_deref())?;
    if let Some(remote) = &a.remote {
        let mut page = remote
            .contacts(Some(&id), "", 1, include)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        return crate::inbox::remote_result(Ok(page["contacts"]
            .as_array_mut()
            .and_then(Vec::pop)
            .ok_or(StatusCode::NOT_FOUND)?));
    }
    a.query(move |db| {
        let mut c = db
            .query_row(
                "SELECT id,first_seen_at,last_seen_at FROM contacts WHERE id=?1",
                [&id],
                contact,
            )
            .optional()?;
        if include && let Some(c) = &mut c {
            c.messages = Some(messages(db, &id, "", 5)?)
        }
        Ok(c)
    })
    .await?
    .map(Json)
    .ok_or(StatusCode::NOT_FOUND)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    limit: Option<usize>,
    cursor: Option<uuid::Uuid>,
}
async fn history(
    State(a): State<Archive>,
    Path(id): Path<String>,
    Query(q): Query<Page>,
) -> Result<Json<MessagePage>> {
    let id = digest(&id)?;
    let n = limit(q.limit)?;
    if let Some(remote) = &a.remote {
        return crate::inbox::remote_result(Ok(remote
            .history(&id, &q.cursor.map(|u| u.to_string()).unwrap_or_default(), n)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::NOT_FOUND)?));
    }
    a.query(move |db| {
        if !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM contacts WHERE id=?1)",
            [&id],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(None);
        }
        messages(
            db,
            &id,
            &q.cursor.map(|v| v.to_string()).unwrap_or_default(),
            n,
        )
        .map(Some)
    })
    .await?
    .map(Json)
    .ok_or(StatusCode::NOT_FOUND)
}
pub fn routes(prefix: &str) -> Router<Archive> {
    Router::new()
        .route(&format!("{prefix}/contacts"), get(list))
        .route(&format!("{prefix}/contacts/{{id}}"), get(detail))
        .route(&format!("{prefix}/contacts/{{id}}/messages"), get(history))
}
