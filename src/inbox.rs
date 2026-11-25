use crate::Archive;
use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use uuid::Uuid;

type ApiResult<T> = Result<T, StatusCode>;

impl Archive {
    fn app_token(&self, headers: &HeaderMap, scope: &str) -> bool {
        let Some(token) = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.as_bytes().strip_prefix(b"Bearer "))
        else {
            return false;
        };
        token.len() == 64
            && self
                .db
                .lock()
                .ok()
                .is_some_and(|db| crate::app_tokens::accepts(&db, scope, token).unwrap_or(false))
    }
    pub(super) fn is_owner(&self, headers: &HeaderMap) -> bool {
        self.owner.as_ref().is_some_and(|expected| {
            headers
                .get(header::AUTHORIZATION)
                .is_some_and(|given| bool::from(expected.as_bytes().ct_eq(given.as_bytes())))
        }) || self.app_token(headers, "api")
    }

    pub(crate) async fn query<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> ApiResult<T> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let conn = db.lock().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
            operation(&conn).map_err(|_| {
                eprintln!("Archive query failed");
                StatusCode::SERVICE_UNAVAILABLE
            })
        })
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    }
}

async fn authorize(
    State(archive): State<Archive>,
    request: Request,
    next: Next,
) -> ApiResult<Response> {
    if !archive.is_owner(request.headers()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    Ok(response)
}

pub(super) fn routes(prefix: &str, archive: Archive) -> Router<Archive> {
    Router::new()
        .route(&format!("{prefix}/messages"), get(list))
        .route(
            &format!("{prefix}/messages/{{id}}"),
            get(detail).patch(update).delete(remove),
        )
        .route(
            &format!("{prefix}/providers"),
            get(|State(archive): State<Archive>| async move {
                Json(archive.providers.as_ref().clone())
            }),
        )
        .merge(crate::webhooks::routes(prefix))
        .merge(crate::contacts::routes(prefix))
        .route_layer(middleware::from_fn_with_state(archive, authorize))
        .route(&format!("{prefix}/otp/recent"), get(recent_for_browser))
        .route(
            &format!("{prefix}/replicas/{{source}}/messages"),
            get(replica_list),
        )
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Entry {
    id: String,
    sender: String,
    body: String,
    archived_at: i64,
    starred: bool,
}
pub(crate) const SELECT: &str = "SELECT m.id, m.sender, m.body, m.archived_at, COALESCE(n.starred, 0), m.rowid FROM messages m LEFT JOIN message_flags n ON n.message_id=m.id";
pub(crate) fn entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        sender: row.get(1)?,
        body: row.get(2)?,
        archived_at: row.get(3)?,
        starred: row.get(4)?,
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    cursor: Option<i64>,
    limit: Option<usize>,
}
#[derive(Serialize, Deserialize)]
struct Listing {
    messages: Vec<Entry>,
    next_cursor: Option<i64>,
}
async fn list(
    State(archive): State<Archive>,
    Query(page): Query<Page>,
) -> ApiResult<Json<Listing>> {
    let limit = page.limit.unwrap_or(30);
    if !(1..=100).contains(&limit) || page.cursor.is_some_and(|n| n <= 0) {
        return Err(StatusCode::BAD_REQUEST);
    }

    if let Some(remote) = &archive.remote {
        return remote_result(
            remote
                .messages(page.cursor.unwrap_or(i64::MAX), limit)
                .await,
        );
    }
    archive
        .query(move |db| {
            let mut stmt = db.prepare(&format!(
                "{SELECT} WHERE m.rowid < ?1 ORDER BY m.rowid DESC LIMIT ?2"
            ))?;
            let mut rows =
                stmt.query(params![page.cursor.unwrap_or(i64::MAX), (limit + 1) as i64])?;
            let mut messages = Vec::new();
            let mut last = 0;
            while let Some(row) = rows.next()? {
                if messages.len() == limit {
                    return Ok(Listing {
                        messages,
                        next_cursor: Some(last),
                    });
                }
                last = row.get(5)?;
                messages.push(entry(row)?);
            }
            Ok(Listing {
                messages,
                next_cursor: None,
            })
        })
        .await
        .map(Json)
}
async fn detail(State(archive): State<Archive>, Path(id): Path<Uuid>) -> ApiResult<Json<Entry>> {
    if let Some(remote) = &archive.remote {
        return remote_result(Ok(remote
            .message(&id.to_string())
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::NOT_FOUND)?));
    }

    archive
        .query(move |db| {
            db.query_row(&format!("{SELECT} WHERE m.id=?1"), [id.to_string()], entry)
                .optional()
        })
        .await?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    starred: bool,
}
async fn update(
    State(archive): State<Archive>,
    Path(id): Path<Uuid>,
    Json(edit): Json<Edit>,
) -> ApiResult<StatusCode> {
    if let Some(remote) = &archive.remote {
        return found(usize::from(
            remote
                .edit(&id.to_string(), Some(edit.starred))
                .await
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
        ));
    }
    let changed=archive.query(move |db| db.execute("INSERT INTO message_flags(message_id,starred) SELECT id,?2 FROM messages WHERE id=?1 ON CONFLICT(message_id) DO UPDATE SET starred=excluded.starred",params![id.to_string(),edit.starred])).await?;
    found(changed)
}
async fn remove(State(archive): State<Archive>, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
    if let Some(remote) = &archive.remote {
        return found(usize::from(
            remote
                .edit(&id.to_string(), None)
                .await
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
        ));
    }
    found(
        archive
            .query(move |db| db.execute("DELETE FROM messages WHERE id=?1", [id.to_string()]))
            .await?,
    )
}
fn found(count: usize) -> ApiResult<StatusCode> {
    if count == 0 {
        Err(StatusCode::NOT_FOUND)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

async fn replica_list(
    State(archive): State<Archive>,
    headers: HeaderMap,
    Path(source): Path<String>,
    Query(page): Query<Page>,
) -> ApiResult<Json<Listing>> {
    if !archive.replica_reader.as_ref().is_some_and(|reader| {
        headers
            .get(header::AUTHORIZATION)
            .is_some_and(|v| bool::from(reader.as_bytes().ct_eq(v.as_bytes())))
    }) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let limit = page.limit.unwrap_or(30);
    if !(1..=100).contains(&limit) || page.cursor.is_some_and(|v| v <= 0) {
        return Err(StatusCode::BAD_REQUEST);
    }
    archive.query(move |db| {
        if !db.table_exists(None, "replicated_messages")? {return Ok(None)}
        let mut stmt=db.prepare("SELECT id,sender,body,archived_at,starred,rowid FROM replicated_messages WHERE region=?1 AND body IS NOT NULL AND rowid<?2 ORDER BY rowid DESC LIMIT ?3")?;
        let rows=stmt.query_map(params![source,page.cursor.unwrap_or(i64::MAX),(limit+1) as i64], |r|Ok((r.get::<_,i64>(5)?,entry(r)?)))?;
        let mut messages=Vec::new();
        let mut last=None;
        for row in rows { let (cursor,message)=row?;
            if messages.len()==limit{return Ok(Some(Listing{messages,next_cursor:last}))}
            last=Some(cursor);messages.push(message);
        }
        Ok(Some(Listing{messages,next_cursor:None}))
    }).await?.map(Json).ok_or(StatusCode::NOT_FOUND)
}

async fn recent_for_browser(
    State(a): State<Archive>,
    headers: HeaderMap,
) -> ApiResult<impl axum::response::IntoResponse> {
    if !a.app_token(&headers, "browser") {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let rows = if let Some(remote) = &a.remote {
        remote
            .recent()
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    } else {
        a.query(|db|db.prepare("SELECT body,archived_at FROM messages WHERE sender='papamo:encrypted:v1' AND archived_at BETWEEN unixepoch()-300 AND unixepoch() ORDER BY rowid DESC LIMIT 25")?.query_map([],|r|Ok(serde_json::json!({"body":r.get::<_,String>(0)?,"archived_at":r.get::<_,i64>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()).await?
    };
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(rows)))
}

pub(crate) fn remote_result<T: serde::de::DeserializeOwned>(
    result: crate::storage::Result<serde_json::Value>,
) -> ApiResult<Json<T>> {
    serde_json::from_value(result.map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?)
        .map(Json)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}
