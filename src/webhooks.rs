use crate::{Archive, hook_store};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Destination {
    url: String,
}
type Result<T> = std::result::Result<T, StatusCode>;
async fn list(State(a): State<Archive>) -> Result<Json<Value>> {
    let items = a
        .query(hook_store::list)
        .await?
        .into_iter()
        .map(|(id, url)| json!({ "id":id,"url":url }))
        .collect::<Vec<_>>();
    Ok(Json(json!({"webhooks":items})))
}
async fn create(
    State(a): State<Archive>,
    Json(d): Json<Destination>,
) -> Result<(StatusCode, Json<Value>)> {
    if !hook_store::valid_url(&d.url) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (id, token) = a
        .query(move |db| match hook_store::create(db, &d.url) {
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(None)
            }
            result => result.map(Some),
        })
        .await?
        .ok_or(StatusCode::CONFLICT)?;
    Ok((StatusCode::CREATED, Json(json!({"id":id,"token":token}))))
}
async fn rotate(State(a): State<Archive>, Path(id): Path<Uuid>) -> Result<Json<Value>> {
    let token = a
        .query(move |db| hook_store::rotate(db, &id.to_string()))
        .await?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({"id":id,"token":token})))
}
async fn remove(State(a): State<Archive>, Path(id): Path<Uuid>) -> Result<StatusCode> {
    if a.query(move |db| hook_store::remove(db, &id.to_string()))
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
pub fn routes(prefix: &str) -> Router<Archive> {
    Router::new()
        .route(&format!("{prefix}/webhooks"), get(list).post(create))
        .route(&format!("{prefix}/webhooks/{{id}}/rotate"), post(rotate))
        .route(&format!("{prefix}/webhooks/{{id}}"), delete(remove))
}
async fn queue_external(a: &Archive, target: &crate::storage::Target) -> Result<()> {
    let after = a
        .query(|db| {
            db.query_row(
                "SELECT revision FROM external_hook_cursor WHERE id=1",
                [],
                |r| r.get(0),
            )
        })
        .await?;
    let events = target
        .events(after)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if events.is_empty() {
        return Ok(());
    }
    a.query(move |db| {
        let tx = db.unchecked_transaction()?;
        for row in events {
            let revision = row["revision"].as_i64().ok_or(rusqlite::Error::InvalidQuery)?;
            let id = row["message_id"].as_str().ok_or(rusqlite::Error::InvalidQuery)?;
            let kind = row["event_type"].as_str().ok_or(rusqlite::Error::InvalidQuery)?;
            if tx.execute("UPDATE external_hook_cursor SET revision=?1 WHERE id=1 AND revision<?1", [revision])? == 1 && kind != "migration" {
                tx.execute("INSERT INTO webhook_deliveries(endpoint,message_id,event) SELECT id,?1,?2 FROM webhook_endpoints", [id,kind])?;
            }
        }
        tx.commit()
    }).await
}
pub fn start(a: Archive) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .timeout(Duration::from_secs(8))
        .build()?;
    tokio::spawn(async move {
        loop {
            if let Some(target) = &a.remote
                && queue_external(&a, target).await.is_err()
            {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            let pending=a.query(|db|db.query_row("SELECT d.id,d.message_id,d.event,e.url,e.bearer FROM webhook_deliveries d JOIN webhook_endpoints e ON e.id=d.endpoint WHERE d.due<=unixepoch() ORDER BY d.due,d.id LIMIT 1",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?))).optional()).await;
            let Ok(Some((id, message, event, url, token))) = pending else {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            };
            let delivered = http
                .post(url)
                .bearer_auth(token)
                .json(&json!({"delivery_id":id,"event":event,"message_id":message}))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success());
            let result=a.query(move|db|if delivered{db.execute("DELETE FROM webhook_deliveries WHERE id=?1",[id])}else{db.execute("UPDATE webhook_deliveries SET due=unixepoch()+min(3600,10*(1<<min(attempts,9))),attempts=attempts+1 WHERE id=?1",[id])}).await;
            if result.is_err() {
                tokio::time::sleep(Duration::from_secs(5)).await
            }
        }
    });
    Ok(())
}
