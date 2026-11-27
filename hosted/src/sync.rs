//! Connects a self-hosted archive to papamo.dev (Pro). The self-hosted service pushes its sealed
//! messages here as an off-site backup and for the web inbox, and pulls down what phones sent
//! here, so phones need no tunnel to reach it. Everything stays sealed with the archive key.
use crate::{App, DOWN, Failure, Reply, encryption, identity, plans};
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::json;

async fn allowed(a: &App, h: &HeaderMap) -> Result<i64, Failure> {
    let user = identity(a, h, false).await?;
    if plans::gates(a, user).await?.self_host_sync {
        Ok(user)
    } else {
        Err(StatusCode::PAYMENT_REQUIRED)
    }
}

#[derive(Deserialize)]
pub struct Item {
    id: uuid::Uuid,
    sender: String,
    body: String,
    contact_id: Option<String>,
    archived_at: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    messages: Vec<Item>,
}
/// Stores up to 200 sealed messages with their original times. Known IDs are skipped.
pub async fn push(State(a): State<App>, h: HeaderMap, Json(batch): Json<Batch>) -> Reply {
    let user = allowed(&a, &h).await?;
    if batch.messages.len() > 200
        || batch.messages.iter().any(|m| {
            !encryption::accepts(&m.sender, &m.body)
                || m.contact_id
                    .as_ref()
                    .is_some_and(|c| c.len() != 64 || !c.bytes().all(|b| b.is_ascii_hexdigit()))
        })
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let mut db = a.pool.get().await.map_err(|_| DOWN)?;
    let tx = db.transaction().await.map_err(|_| DOWN)?;
    let mut stored = 0u64;
    for m in &batch.messages {
        stored += tx
            .execute(
                "INSERT INTO archived_messages(account_id,id,sender,body,contact_id,received_at)
                 VALUES($1,$2,$3,$4,$5,to_timestamp($6)) ON CONFLICT DO NOTHING",
                &[
                    &user,
                    &m.id.to_string(),
                    &m.sender,
                    &m.body,
                    &m.contact_id.as_ref().map(|c| c.to_ascii_lowercase()),
                    &(m.archived_at as f64),
                ],
            )
            .await
            .map_err(|_| DOWN)?;
    }
    if stored > 0 {
        tx.execute("SELECT pg_notify('psms_arrival', $1)", &[&user.to_string()])
            .await
            .map_err(|_| DOWN)?;
    }
    tx.commit().await.map_err(|_| DOWN)?;
    Ok(Json(json!({"stored": stored})))
}

#[derive(Deserialize)]
pub struct Cursor {
    /// Microseconds since the epoch of the last message already pulled.
    after: Option<i64>,
    after_id: Option<String>,
    limit: Option<i64>,
}
/// Messages in arrival order after a cursor, for the self-hosted service to copy down.
pub async fn pull(State(a): State<App>, h: HeaderMap, Query(c): Query<Cursor>) -> Reply {
    let user = allowed(&a, &h).await?;
    let limit = c.limit.unwrap_or(200).clamp(1, 500);
    let rows = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query(
            "SELECT id,sender,body,contact_id,(extract(epoch FROM received_at)*1000000)::bigint
             FROM archived_messages WHERE account_id=$1
               AND ((extract(epoch FROM received_at)*1000000)::bigint, id) > ($2, $3)
             ORDER BY received_at, id LIMIT $4",
            &[
                &user,
                &c.after.unwrap_or(0),
                &c.after_id.unwrap_or_default(),
                &limit,
            ],
        )
        .await
        .map_err(|_| DOWN)?;
    let last = rows
        .last()
        .map(|r| json!({"after": r.get::<_, i64>(4), "after_id": r.get::<_, String>(0)}));
    Ok(Json(json!({
        "messages": rows.iter().map(|r| json!({
            "id": r.get::<_, String>(0), "sender": r.get::<_, String>(1), "body": r.get::<_, String>(2),
            "contact_id": r.get::<_, Option<String>>(3), "archived_at": r.get::<_, i64>(4) / 1_000_000,
        })).collect::<Vec<_>>(),
        "next": last,
        "more": rows.len() as i64 == limit,
    })))
}
