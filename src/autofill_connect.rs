//! pSMS Autofill asks the pSMS app on this computer for a connection, and the app asks you.
//! Only a browser extension can start a request (websites can't send an extension Origin), and
//! nothing is handed over until you click Allow in pSMS. The app then leaves a browser token and
//! the archive key here once, for that extension to collect.
use crate::Archive;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS autofill_requests(
    id TEXT PRIMARY KEY NOT NULL, origin TEXT NOT NULL, created INTEGER NOT NULL DEFAULT (unixepoch()),
    status TEXT NOT NULL DEFAULT 'waiting' CHECK(status IN ('waiting','approved','denied')),
    grant TEXT)";
/// How long a request waits for you in pSMS.
const WAIT_SECS: i64 = 300;

/// The extension that's asking, from its Origin. Web pages can't send these.
fn extension(headers: &HeaderMap) -> Option<String> {
    let origin = headers.get("origin")?.to_str().ok()?;
    [
        "chrome-extension://",
        "moz-extension://",
        "safari-web-extension://",
    ]
    .iter()
    .any(|p| origin.starts_with(p) && origin.len() > p.len() && origin.len() < 200)
    .then(|| origin.to_string())
}

async fn ask(
    State(archive): State<Archive>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let origin = extension(&headers).ok_or(StatusCode::FORBIDDEN)?;
    let id = uuid::Uuid::new_v4().to_string();
    let saved = id.clone();
    tokio::task::spawn_blocking(move || {
        let db = archive
            .db
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        db.execute(
            "DELETE FROM autofill_requests WHERE created < unixepoch() - ?1",
            [WAIT_SECS],
        )
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let waiting: i64 = db
            .query_row(
                "SELECT count(*) FROM autofill_requests WHERE status='waiting'",
                [],
                |r| r.get(0),
            )
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if waiting >= 3 {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        db.execute(
            "INSERT INTO autofill_requests(id,origin) VALUES(?1,?2)",
            params![saved, origin],
        )
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        Ok(())
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)??;
    archive.arrivals.send_replace(());
    Ok(Json(json!({"id": id, "status": "waiting"})))
}

/// The extension checks back; an approved grant is handed over once and then deleted.
async fn check(
    State(archive): State<Archive>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let origin = extension(&headers).ok_or(StatusCode::FORBIDDEN)?;
    tokio::task::spawn_blocking(move || {
        let db = archive.db.lock().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let row: Option<(String, Option<String>)> = db
            .query_row(
                "SELECT status, grant FROM autofill_requests WHERE id=?1 AND origin=?2 AND created >= unixepoch() - ?3",
                params![id, origin, WAIT_SECS],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let (status, grant) = row.ok_or(StatusCode::NOT_FOUND)?;
        if status != "waiting" {
            db.execute("DELETE FROM autofill_requests WHERE id=?1", [&id])
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        }
        let mut out = json!({"status": status});
        if let Some(grant) = grant.and_then(|g| serde_json::from_str::<Value>(&g).ok()) {
            out["grant"] = grant;
        }
        Ok(Json(out))
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}

pub fn routes(prefix: &str, archive: Archive) -> Router<Archive> {
    Router::new()
        .route(&format!("{prefix}/autofill/connect"), post(ask))
        .route(&format!("{prefix}/autofill/connect/{{id}}"), get(check))
        .with_state(archive)
}
