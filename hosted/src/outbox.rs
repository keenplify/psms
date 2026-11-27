//! Texts queued in an inbox for a sending device, matching the local archive's outbox. Each one
//! is sealed with the archive key in the browser (`{"to","body"}`), so the server stores only the
//! envelope until a device reports it sent, then drops it.
use crate::{App, DOWN, Failure, Reply, encryption, identity};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Queue {
    envelope: String,
    /// Send at this Unix time instead of now, like BlueBubbles' scheduled messages.
    #[serde(default)]
    send_at: Option<i64>,
}
pub async fn queue(
    State(a): State<App>,
    h: HeaderMap,
    Json(q): Json<Queue>,
) -> Result<(StatusCode, Json<serde_json::Value>), Failure> {
    let user = identity(&a, &h, false).await?;
    if !crate::plans::gates(&a, user).await?.replies {
        return Err(StatusCode::PAYMENT_REQUIRED);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    if q.envelope.len() > 16_000
        || !encryption::accepts(encryption::ACTIONS, &q.envelope)
        || q.send_at.is_some_and(|t| t > now + 366 * 24 * 3600)
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let send_at = q.send_at.filter(|t| *t > now).map(|t| t as f64);
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let waiting: i64 = db
        .query_one(
            "SELECT count(*) FROM outbox WHERE account_id=$1 AND status IN ('queued','sending')",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?
        .get(0);
    if waiting >= 100 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let id = Uuid::new_v4();
    db.execute(
        "INSERT INTO outbox(id,account_id,envelope,send_at) VALUES($1,$2,$3,to_timestamp($4))",
        &[&id.to_string(), &user, &q.envelope, &send_at],
    )
    .await
    .map_err(|_| DOWN)?;
    db.execute("SELECT pg_notify('psms_arrival', $1)", &[&user.to_string()])
        .await
        .map_err(|_| DOWN)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"id": id, "status": "queued", "send_at": q.send_at.filter(|t| *t > now)})),
    ))
}
pub async fn status(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Reply {
    let user = identity(&a, &h, false).await?;
    let row = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query_opt(
            "SELECT status,error,floor(extract(epoch FROM created_at))::bigint,floor(extract(epoch FROM finished_at))::bigint
             FROM outbox WHERE account_id=$1 AND id=$2",
            &[&user, &id.to_string()],
        )
        .await
        .map_err(|_| DOWN)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({
        "id": id, "status": row.get::<_, String>(0), "error": row.get::<_, Option<String>>(1),
        "created_at": row.get::<_, i64>(2), "finished_at": row.get::<_, Option<i64>>(3),
    })))
}
/// Waiting and recent texts, newest first, so an inbox can show their progress.
pub async fn list(State(a): State<App>, h: HeaderMap) -> Reply {
    let user = identity(&a, &h, false).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let rows = db
        .query(
            "SELECT id,status,error,envelope,floor(extract(epoch FROM created_at))::bigint,floor(extract(epoch FROM send_at))::bigint FROM outbox
             WHERE account_id=$1 AND (status IN ('queued','sending','failed') OR finished_at > now() - interval '1 day')
             ORDER BY created_at DESC LIMIT 50",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?;
    let seen: Option<i64> = db
        .query_one(
            "SELECT floor(extract(epoch FROM max(last_used_at)))::bigint FROM sessions
             WHERE account_id=$1 AND scope='computer' AND expires_at>now()",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?
        .get(0);
    Ok(Json(json!({"messages": rows.iter().map(|r| json!({
        "id": r.get::<_, String>(0), "status": r.get::<_, String>(1), "error": r.get::<_, Option<String>>(2),
        "envelope": r.get::<_, Option<String>>(3), "created_at": r.get::<_, i64>(4),
        "send_at": r.get::<_, Option<i64>>(5),
    })).collect::<Vec<_>>(), "computer_seen_at": seen})))
}
pub async fn claim(State(a): State<App>, h: HeaderMap) -> Reply {
    let user = identity(&a, &h, false).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    // The inbox shows when the sending Mac last checked in, so a waiting reply explains itself.
    db.execute(
        "UPDATE sessions SET last_used_at=now() WHERE digest=$1 AND scope='computer'
           AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')",
        &[&crate::bearer(&h)?],
    )
    .await
    .map_err(|_| DOWN)?;
    db.execute(
        "UPDATE outbox SET status='queued', claimed_at=NULL
         WHERE account_id=$1 AND status='sending' AND claimed_at < now() - interval '5 minutes'",
        &[&user],
    )
    .await
    .map_err(|_| DOWN)?;
    let rows = db
        .query(
            "UPDATE outbox SET status='sending', claimed_at=now()
             WHERE account_id=$1 AND id IN (SELECT id FROM outbox WHERE account_id=$1 AND status='queued'
                 AND (send_at IS NULL OR send_at <= now())
                 ORDER BY coalesce(send_at, created_at) LIMIT 10 FOR UPDATE SKIP LOCKED)
             RETURNING id, envelope",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?;
    // Lets the Mac fetch messages only when something new arrived (for one-time code alerts).
    let newest: Option<i64> = db
        .query_one(
            "SELECT floor(extract(epoch FROM max(received_at)))::bigint FROM archived_messages WHERE account_id=$1",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?
        .get(0);
    Ok(Json(json!({"messages": rows.iter().map(|r| json!({
        "id": r.get::<_, String>(0), "envelope": r.get::<_, Option<String>>(1),
    })).collect::<Vec<_>>(), "newest": newest})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    status: String,
    error: Option<String>,
}
pub async fn result(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(o): Json<Outcome>,
) -> Result<StatusCode, Failure> {
    let user = identity(&a, &h, false).await?;
    if !matches!(o.status.as_str(), "sent" | "failed") {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let error = o.error.map(|e| e.chars().take(300).collect::<String>());
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let changed = db
        .execute(
            "UPDATE outbox SET status=$3, error=$4, finished_at=now(),
                envelope=CASE WHEN $3='sent' THEN NULL ELSE envelope END
             WHERE account_id=$1 AND id=$2 AND status='sending'",
            &[&user, &id.to_string(), &o.status, &error],
        )
        .await
        .map_err(|_| DOWN)?;
    if changed == 0 {
        return Err(StatusCode::CONFLICT);
    }
    db.execute("SELECT pg_notify('psms_arrival', $1)", &[&user.to_string()])
        .await
        .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
/// Removes a failed or waiting text.
pub async fn cancel(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Failure> {
    let user = identity(&a, &h, false).await?;
    match a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .execute(
            "DELETE FROM outbox WHERE account_id=$1 AND id=$2 AND status IN ('queued','failed')",
            &[&user, &id.to_string()],
        )
        .await
        .map_err(|_| DOWN)?
    {
        0 => Err(StatusCode::CONFLICT),
        _ => Ok(StatusCode::NO_CONTENT),
    }
}
