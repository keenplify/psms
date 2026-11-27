mod browser;
mod outbox;
mod plans;
mod sync;
mod teams;
mod webhooks;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post, put},
};
use deadpool_postgres::{Manager, Pool};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio_postgres::{Config, NoTls};
use uuid::Uuid;

#[macro_export]
macro_rules! env_name {
    ($name:literal) => {
        concat!("PSMS_", $name)
    };
}
#[allow(dead_code)]
#[path = "../../src/encryption.rs"]
mod encryption;

pub(crate) type Failure = StatusCode;
pub(crate) type Reply = Result<Json<Value>, Failure>;
pub(crate) const DOWN: Failure = StatusCode::SERVICE_UNAVAILABLE;
const DENIED: Failure = StatusCode::UNAUTHORIZED;

#[derive(Clone)]
pub(crate) struct App {
    pub(crate) arrivals: tokio::sync::broadcast::Sender<Option<i64>>,
    pub(crate) pool: Pool,
    pub(crate) plans: std::sync::Arc<plans::Cache>,
    exchange_key: String,
}
#[derive(Deserialize)]
struct SharedIdentity {
    subject: Uuid,
    username: String,
}
#[derive(Deserialize)]
struct Message {
    id: Uuid,
    sender: String,
    body: String,
    contact_id: Option<String>,
    /// The conversation, an opaque ID shared with `/chats`.
    #[serde(default)]
    chat_id: Option<String>,
}
fn valid_chat(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn bearer(headers: &HeaderMap) -> Result<Vec<u8>, Failure> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| v.len() == 64)
        .ok_or(DENIED)?;
    Ok(Sha256::digest(token.as_bytes()).to_vec())
}
/// Free accounts get 100 API calls per 30 minutes on the public API: computer tokens made for
/// other services. pSMS's own apps are never counted: the signed-in web inbox, phone uploads,
/// and computer tokens that pSMS for Mac or self-hosted Plus sync asked for. Unlimited plans
/// and self-hosted servers have no limit.
async fn throttle(
    State(a): State<App>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = request.uri().path();
    let relay_poll = path.ends_with("/outbox/claim") || path.ends_with("/result");
    let Ok(digest) = bearer(request.headers()) else {
        return next.run(request).await;
    };
    if relay_poll {
        return next.run(request).await;
    }
    let Ok(db) = a.pool.get().await else {
        return DOWN.into_response();
    };
    let counted = db
        .query_opt(
            "INSERT INTO api_usage(account_id,window_start,calls)
               SELECT s.account_id, now(), 1 FROM sessions s JOIN archive_quota q ON q.id=s.account_id
               WHERE s.digest=$1 AND s.scope='computer' AND s.client IS NULL AND s.expires_at>now() AND q.api_calls_per_30_min IS NOT NULL
             ON CONFLICT(account_id) DO UPDATE SET
               calls=CASE WHEN api_usage.window_start <= now()-interval '30 minutes' THEN 1 ELSE api_usage.calls+1 END,
               window_start=CASE WHEN api_usage.window_start <= now()-interval '30 minutes' THEN now() ELSE api_usage.window_start END
             RETURNING calls::bigint, ceil(extract(epoch FROM window_start + interval '30 minutes' - now()))::bigint,
               (SELECT api_calls_per_30_min::bigint FROM archive_quota WHERE id=api_usage.account_id)",
            &[&digest],
        )
        .await;
    drop(db);
    let Some(row) = (match counted {
        Ok(row) => row,
        Err(_) => return DOWN.into_response(),
    }) else {
        return next.run(request).await;
    };
    let (calls, reset, limit): (i64, i64, i64) = (
        row.get(0),
        row.get::<_, i64>(1).max(1),
        row.get::<_, Option<i64>>(2).unwrap_or(i64::MAX),
    );
    let mut response = if calls > limit {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": format!("This account has used its {limit} API calls for the last 30 minutes. Upgrade your plan to remove the limit.")})),
        )
            .into_response()
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    for (name, value) in [
        ("x-ratelimit-limit", limit),
        ("x-ratelimit-remaining", (limit - calls).max(0)),
        ("x-ratelimit-reset", reset),
    ] {
        headers.insert(name, value.into());
    }
    if calls > limit {
        headers.insert("retry-after", reset.into());
    }
    response
}
/// The account a request acts on. Team members name the shared inbox with `x-psms-inbox`;
/// they act on it only while they are members and its plan includes teams.
pub(crate) async fn identity(app: &App, h: &HeaderMap, upload: bool) -> Result<i64, Failure> {
    let db = app.pool.get().await.map_err(|_| DOWN)?;
    let me: i64 = db.query_opt("SELECT account_id FROM sessions WHERE digest=$1 AND expires_at>now() AND (scope IN ('owner','computer') OR $2)",&[&bearer(h)?,&upload]).await.map_err(|_|DOWN)?.map(|r|r.get(0)).ok_or(DENIED)?;
    let Some(inbox) = h
        .get("x-psms-inbox")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|inbox| *inbox != me)
    else {
        return Ok(me);
    };
    db.query_opt(
        "SELECT 1 FROM account_members m JOIN archive_quota q ON q.id=m.owner_id
         WHERE m.owner_id=$1 AND m.member_id=$2 AND q.teams",
        &[&inbox, &me],
    )
    .await
    .map_err(|_| DOWN)?
    .map(|_| inbox)
    .ok_or(StatusCode::FORBIDDEN)
}
fn new_token() -> (String, Vec<u8>) {
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let digest = Sha256::digest(token.as_bytes()).to_vec();
    (token, digest)
}
async fn exchange(
    State(app): State<App>,
    h: HeaderMap,
    Json(identity): Json<SharedIdentity>,
) -> Reply {
    let actual = bearer(&h)?;
    let expected = Sha256::digest(app.exchange_key.as_bytes());
    if !bool::from(actual.as_slice().ct_eq(expected.as_slice())) {
        return Err(DENIED);
    }
    if identity.username.is_empty() || identity.username.len() > 128 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut db = app.pool.get().await.map_err(|_| DOWN)?;
    let tx = db.transaction().await.map_err(|_| DOWN)?;
    let subject = identity.subject.to_string();
    let id:i64=tx.query_one("INSERT INTO accounts(username,sg_subject,display_name) VALUES($1,$1,$2) ON CONFLICT(sg_subject) DO UPDATE SET display_name=excluded.display_name RETURNING id",&[&subject,&identity.username]).await.map_err(|_|DOWN)?.get(0);
    let (token, digest) = new_token();
    tx.execute(
        "DELETE FROM sessions WHERE account_id=$1 AND expires_at<=now()",
        &[&id],
    )
    .await
    .map_err(|_| DOWN)?;
    tx.execute(
        "INSERT INTO sessions VALUES($1,$2,now()+interval '1 day','owner')",
        &[&digest, &id],
    )
    .await
    .map_err(|_| DOWN)?;
    tx.commit().await.map_err(|_| DOWN)?;
    Ok(Json(json!({"token":token})))
}
async fn logout(State(a): State<App>, h: HeaderMap) -> Result<StatusCode, Failure> {
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    db.execute("DELETE FROM sessions WHERE digest=$1", &[&bearer(&h)?])
        .await
        .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn me(State(a): State<App>, h: HeaderMap) -> Reply {
    let id = identity(&a, &h, false).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let row=db.query_one("SELECT coalesce(display_name,username),(SELECT count(*) FROM archived_messages WHERE account_id=$1),q.message_limit,q.plan,q.plan_name,q.api_calls_per_30_min,q.webhooks,q.admin,q.max_phones,q.replies,q.autofill FROM accounts a JOIN archive_quota q USING(id) WHERE a.id=$1",&[&id]).await.map_err(|_|DOWN)?;
    Ok(Json(
        json!({"username":row.get::<_,String>(0),"used":row.get::<_,i64>(1),"limit":row.get::<_,Option<i64>>(2),
            "plan":{"id":row.get::<_,String>(3),"name":row.get::<_,String>(4),"api_calls_per_30_min":row.get::<_,Option<i32>>(5),"webhooks":row.get::<_,bool>(6),
                "max_phones":row.get::<_,Option<i32>>(8),"replies":row.get::<_,bool>(9),"autofill":row.get::<_,bool>(10)},
            "admin":row.get::<_,bool>(7)}),
    ))
}
/// A year-long token for a computer that serves this inbox, such as pSMS for Mac sending
/// queued texts. It can do what a signed-in inbox can, and is listed and removed with phones.
#[derive(Deserialize, Default)]
struct ComputerRequest {
    /// The pSMS app asking, such as "pSMS for Mac". Leave out for a public API token.
    client: Option<String>,
}
/// pSMS's own apps that a computer token can belong to; their calls aren't rate-limited.
const APPS: [&str; 2] = ["pSMS for Mac", "pSMS self-hosted sync"];
async fn computer(
    State(a): State<App>,
    h: HeaderMap,
    body: Option<Json<ComputerRequest>>,
) -> Result<axum::response::Response, Failure> {
    use axum::response::IntoResponse;
    let client = body
        .and_then(|Json(b)| b.client)
        .filter(|c| APPS.contains(&c.as_str()));
    let id = identity(&a, &h, false).await?;
    if !plans::gates(&a, id).await?.replies {
        return Ok(plans::upgrade("Replying from anywhere"));
    }
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let (token, digest) = new_token();
    db.execute(
        "INSERT INTO sessions(digest,account_id,expires_at,scope,client) VALUES($1,$2,now()+interval '1 year','computer',$3)",
        &[&digest, &id, &client],
    )
    .await
    .map_err(|_| DOWN)?;
    Ok(Json(json!({"token":token})).into_response())
}
async fn pair(State(a): State<App>, h: HeaderMap) -> Result<axum::response::Response, Failure> {
    use axum::response::IntoResponse;
    let id = identity(&a, &h, false).await?;
    let gates = plans::gates(&a, id).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    if let Some(max) = gates.max_phones {
        let phones: i64 = db
            .query_one(
                "SELECT count(*) FROM sessions WHERE account_id=$1 AND scope='upload' AND expires_at>now()",
                &[&id],
            )
            .await
            .map_err(|_| DOWN)?
            .get(0);
        if phones >= max as i64 {
            return Ok(plans::upgrade(&format!(
                "Connecting more than {max} phones"
            )));
        }
    }
    let (token, digest) = new_token();
    db.execute(
        "INSERT INTO sessions VALUES($1,$2,now()+interval '1 year','upload')",
        &[&digest, &id],
    )
    .await
    .map_err(|_| DOWN)?;
    Ok(Json(json!({"token":token})).into_response())
}
// The phone's pairing page marks its upload token as opened; the inbox and that page poll the
// same token to see whether it opened the page and stored its first message.
async fn mark_seen(State(a): State<App>, h: HeaderMap) -> Result<StatusCode, Failure> {
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let updated = db
        .execute(
            "UPDATE sessions SET seen_at=now() WHERE digest=$1 AND scope='upload' AND expires_at>now()",
            &[&bearer(&h)?],
        )
        .await
        .map_err(|_| DOWN)?;
    if updated == 1 {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(DENIED)
    }
}
async fn mark_ready(State(a): State<App>, h: HeaderMap) -> Result<StatusCode, Failure> {
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let updated = db
        .execute(
            "UPDATE sessions SET ready_at=now() WHERE digest=$1 AND scope='upload' AND expires_at>now()",
            &[&bearer(&h)?],
        )
        .await
        .map_err(|_| DOWN)?;
    if updated == 1 {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(DENIED)
    }
}
async fn seen(State(a): State<App>, h: HeaderMap) -> Reply {
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let row = db
        .query_opt(
            "SELECT seen_at IS NOT NULL, used_at IS NOT NULL, ready_at IS NOT NULL FROM sessions WHERE digest=$1 AND scope='upload' AND expires_at>now()",
            &[&bearer(&h)?],
        )
        .await
        .map_err(|_| DOWN)?
        .ok_or(DENIED)?;
    Ok(Json(
        json!({"opened": row.get::<_, bool>(0), "used": row.get::<_, bool>(1), "ready": row.get::<_, bool>(2)}),
    ))
}
async fn store(
    State(a): State<App>,
    h: HeaderMap,
    Json(m): Json<Message>,
) -> Result<(StatusCode, Json<Value>), Failure> {
    let user = identity(&a, &h, true).await?;
    if !encryption::accepts(&m.sender, &m.body)
        || m.contact_id
            .as_ref()
            .is_some_and(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
        || m.chat_id
            .as_ref()
            .is_some_and(|c| !valid_chat(&c.to_ascii_lowercase()))
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let id = m.id.to_string();
    let chat = m.chat_id.map(|c| c.to_ascii_lowercase());
    let contact = m.contact_id.map(|s| s.to_ascii_lowercase());
    let mut db = a.pool.get().await.map_err(|_| DOWN)?;
    let tx = db.transaction().await.map_err(|_| DOWN)?;
    tx.query_one("SELECT id FROM accounts WHERE id=$1 FOR UPDATE", &[&user])
        .await
        .map_err(|_| DOWN)?;
    if let Some(row)=tx.query_opt("SELECT sender=$3 AND body=$4 AND contact_id IS NOT DISTINCT FROM $5 FROM archived_messages WHERE account_id=$1 AND id=$2",&[&user,&id,&m.sender,&m.body,&contact]).await.map_err(|_|DOWN)?{
 return if row.get::<_,bool>(0){Ok((StatusCode::OK, Json(card_hint(&tx, user, contact.as_deref()).await?)))}else{Err(StatusCode::CONFLICT)}
 }
    // A full free archive makes room by deleting its oldest messages. Starred ones are kept.
    let excess: i64 = tx.query_one(
        "SELECT CASE WHEN message_limit IS NULL THEN 0 ELSE
         greatest((SELECT count(*) FROM archived_messages WHERE account_id=$1) - message_limit + 1, 0)
         END FROM archive_quota WHERE id=$1", &[&user]
    ).await.map_err(|_|DOWN)?.get(0);
    if excess > 0 {
        let removed = tx
            .execute(
                "DELETE FROM archived_messages WHERE account_id=$1 AND id IN (
                   SELECT id FROM archived_messages WHERE account_id=$1 AND NOT starred
                   ORDER BY received_at, id LIMIT $2)",
                &[&user, &excess],
            )
            .await
            .map_err(|_| DOWN)?;
        if (removed as i64) < excess {
            return Err(StatusCode::INSUFFICIENT_STORAGE);
        }
    }
    tx.execute("INSERT INTO archived_messages(account_id,id,sender,body,contact_id,chat_id) VALUES($1,$2,$3,$4,$5,$6)",&[&user,&id,&m.sender,&m.body,&contact,&chat]).await.map_err(|_|DOWN)?;
    tx.execute(
        "UPDATE sessions SET used_at=coalesce(used_at,now()), last_used_at=now() WHERE digest=$1 AND scope='upload'",
        &[&bearer(&h)?],
    )
    .await
    .map_err(|_| DOWN)?;
    tx.execute("SELECT pg_notify('psms_arrival', $1)", &[&user.to_string()])
        .await
        .map_err(|_| DOWN)?;
    let hint = card_hint(&tx, user, contact.as_deref()).await?;
    tx.commit().await.map_err(|_| DOWN)?;
    Ok((StatusCode::CREATED, Json(hint)))
}
/// Asks the phone for a contact card when it has none for this sender or its copy is stale.
/// Decided from timestamps alone; the server never reads a card.
async fn card_hint(
    db: &impl deadpool_postgres::GenericClient,
    user: i64,
    contact: Option<&str>,
) -> Result<Value, Failure> {
    let Some(contact) = contact else {
        return Ok(json!({}));
    };
    let fresh: bool = db
        .query_opt(
            "SELECT phone_updated_at > now() - interval '30 days' FROM contact_cards WHERE account_id=$1 AND contact_id=$2",
            &[&user, &contact],
        )
        .await
        .map_err(|_| DOWN)?
        .and_then(|r| r.get::<_, Option<bool>>(0))
        .unwrap_or(false);
    Ok(if fresh {
        json!({})
    } else {
        json!({"contact_card": "wanted"})
    })
}
#[derive(Deserialize)]
struct Card {
    envelope: Option<String>,
    /// "device" stores a card read from a device's address book as the phone's copy.
    #[serde(default)]
    copy: Option<String>,
}
fn valid_contact(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
#[derive(Deserialize)]
struct Attachment {
    envelope: String,
}
/// The largest sealed attachment (about 1.5 MB of file), and all of an account's together.
const ATTACHMENT_MAX: usize = 2 * 1024 * 1024;
const ATTACHMENTS_PER_ACCOUNT: i64 = 100 * 1024 * 1024;
fn attachment_slot(message: &str, index: i16) -> Option<String> {
    (0..10)
        .contains(&index)
        .then(|| Uuid::parse_str(message).ok())
        .flatten()
        .map(|id| id.to_string())
}
#[derive(Deserialize)]
struct Sealed {
    envelope: String,
}
/// A conversation's sealed name, members and Messages identifier, saved by pSMS for Mac.
async fn put_chat(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(chat): Json<Sealed>,
) -> Result<StatusCode, Failure> {
    if !valid_chat(&id)
        || chat.envelope.len() > 64 * 1024
        || !encryption::accepts(encryption::ACTIONS, &chat.envelope)
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let user = identity(&a, &h, true).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    db.execute(
        "INSERT INTO chats(account_id,id,envelope) VALUES($1,$2,$3)
         ON CONFLICT(account_id,id) DO UPDATE SET envelope=excluded.envelope, updated_at=now()",
        &[&user, &id, &chat.envelope],
    )
    .await
    .map_err(|_| DOWN)?;
    db.execute("SELECT pg_notify('psms_arrival', $1)", &[&user.to_string()])
        .await
        .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn list_chats(State(a): State<App>, h: HeaderMap) -> Reply {
    let user = identity(&a, &h, false).await?;
    let rows = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query(
            "SELECT id, envelope, floor(extract(epoch FROM updated_at))::bigint FROM chats WHERE account_id=$1 ORDER BY updated_at DESC",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?;
    Ok(Json(json!({"chats": rows.iter().map(|r| json!({
        "id": r.get::<_, String>(0), "envelope": r.get::<_, String>(1), "updated_at": r.get::<_, i64>(2),
    })).collect::<Vec<_>>()})))
}
fn uuid_text(id: &str) -> Option<String> {
    Uuid::parse_str(id).ok().map(|u| u.to_string())
}
/// A sealed tapback on a message. Reactions hang off their message and don't count as texts.
async fn put_reaction(
    State(a): State<App>,
    h: HeaderMap,
    Path((message, reaction)): Path<(String, String)>,
    Json(sealed): Json<Sealed>,
) -> Result<StatusCode, Failure> {
    let (Some(message), Some(reaction)) = (uuid_text(&message), uuid_text(&reaction)) else {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    };
    if sealed.envelope.len() > 8 * 1024
        || !encryption::accepts(encryption::ACTIONS, &sealed.envelope)
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let user = identity(&a, &h, true).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let count: Option<i64> = db
        .query_opt(
            "SELECT (SELECT count(*) FROM message_reactions WHERE account_id=$1 AND message_id=$2) FROM archived_messages WHERE account_id=$1 AND id=$2",
            &[&user, &message],
        )
        .await
        .map_err(|_| DOWN)?
        .map(|r| r.get(0));
    match count {
        None => return Err(StatusCode::NOT_FOUND),
        Some(n) if n >= 100 => return Err(StatusCode::PAYLOAD_TOO_LARGE),
        _ => {}
    }
    let stored = db
        .execute(
            "INSERT INTO message_reactions(account_id,id,message_id,envelope) VALUES($1,$2,$3,$4)
             ON CONFLICT(account_id,id) DO UPDATE SET envelope=excluded.envelope
             WHERE message_reactions.message_id=excluded.message_id",
            &[&user, &reaction, &message, &sealed.envelope],
        )
        .await
        .map_err(|_| DOWN)?;
    if stored == 0 {
        return Err(StatusCode::CONFLICT);
    }
    db.execute("SELECT pg_notify('psms_arrival', $1)", &[&user.to_string()])
        .await
        .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_reaction(
    State(a): State<App>,
    h: HeaderMap,
    Path((message, reaction)): Path<(String, String)>,
) -> Result<StatusCode, Failure> {
    let (Some(message), Some(reaction)) = (uuid_text(&message), uuid_text(&reaction)) else {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    };
    let user = identity(&a, &h, true).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let removed = db
        .execute(
            "DELETE FROM message_reactions WHERE account_id=$1 AND id=$2 AND message_id=$3",
            &[&user, &reaction, &message],
        )
        .await
        .map_err(|_| DOWN)?;
    if removed == 0 {
        return Err(StatusCode::NOT_FOUND);
    }
    db.execute("SELECT pg_notify('psms_arrival', $1)", &[&user.to_string()])
        .await
        .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
/// pSMS for Mac uploads a photo or file that came with a text, after the text itself.
async fn put_attachment(
    State(a): State<App>,
    h: HeaderMap,
    Path((message, index)): Path<(String, i16)>,
    Json(upload): Json<Attachment>,
) -> Result<StatusCode, Failure> {
    let message = attachment_slot(&message, index)
        .filter(|_| {
            upload.envelope.len() <= ATTACHMENT_MAX
                && encryption::accepts(encryption::ACTIONS, &upload.envelope)
        })
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
    let user = identity(&a, &h, true).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let used: i64 = db
        .query_one(
            "SELECT coalesce(sum(length(envelope)),0)::bigint FROM message_attachments WHERE account_id=$1",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?
        .get(0);
    if used + upload.envelope.len() as i64 > ATTACHMENTS_PER_ACCOUNT {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let stored = db
        .execute(
            "INSERT INTO message_attachments(account_id,message_id,idx,envelope)
             SELECT $1,$2,$3,$4 WHERE EXISTS (SELECT 1 FROM archived_messages WHERE account_id=$1 AND id=$2)
             ON CONFLICT(account_id,message_id,idx) DO UPDATE SET envelope=excluded.envelope",
            &[&user, &message, &index, &upload.envelope],
        )
        .await
        .map_err(|_| DOWN)?;
    if stored == 0 {
        return Err(StatusCode::NOT_FOUND);
    }
    db.execute("SELECT pg_notify('psms_arrival', $1)", &[&user.to_string()])
        .await
        .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn get_attachment(
    State(a): State<App>,
    h: HeaderMap,
    Path((message, index)): Path<(String, i16)>,
) -> Reply {
    let message = attachment_slot(&message, index).ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
    let user = identity(&a, &h, false).await?;
    let row = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query_opt(
            "SELECT envelope FROM message_attachments WHERE account_id=$1 AND message_id=$2 AND idx=$3",
            &[&user, &message, &index],
        )
        .await
        .map_err(|_| DOWN)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({"envelope": row.get::<_, String>(0)})))
}
/// A phone saves its address-book copy with its upload token; an inbox saves edits with the
/// owner session. An empty envelope from an inbox clears the edits.
async fn put_card(
    State(a): State<App>,
    h: HeaderMap,
    Path(contact): Path<String>,
    Json(card): Json<Card>,
) -> Result<StatusCode, Failure> {
    if !valid_contact(&contact)
        || card
            .envelope
            .as_ref()
            .is_some_and(|e| e.len() > 400_000 || !encryption::accepts(encryption::ACTIONS, e))
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let (user, scope): (i64, String) = db
        .query_opt(
            "SELECT account_id, scope FROM sessions WHERE digest=$1 AND expires_at>now()",
            &[&bearer(&h)?],
        )
        .await
        .map_err(|_| DOWN)?
        .map(|r| (r.get(0), r.get(1)))
        .ok_or(DENIED)?;
    let device = card.copy.as_deref() == Some("device");
    let sql = match (scope.as_str(), card.envelope.is_some()) {
        ("upload", true) | ("owner" | "computer", true) if scope == "upload" || device => "INSERT INTO contact_cards(account_id,contact_id,phone_card,phone_updated_at) VALUES($1,$2,$3,now())
             ON CONFLICT(account_id,contact_id) DO UPDATE SET phone_card=excluded.phone_card, phone_updated_at=now()",
        ("owner" | "computer", true) => "INSERT INTO contact_cards(account_id,contact_id,manual_card,manual_updated_at) VALUES($1,$2,$3,now())
             ON CONFLICT(account_id,contact_id) DO UPDATE SET manual_card=excluded.manual_card, manual_updated_at=now()",
        ("owner" | "computer", false) => "UPDATE contact_cards SET manual_card=NULL, manual_updated_at=now() WHERE account_id=$1 AND contact_id=$2 AND $3::text IS NULL",
        _ => return Err(StatusCode::UNPROCESSABLE_ENTITY),
    };
    db.execute(sql, &[&user, &contact, &card.envelope])
        .await
        .map_err(|_| DOWN)?;
    db.execute("SELECT pg_notify('psms_arrival', $1)", &[&user.to_string()])
        .await
        .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn cards(State(a): State<App>, h: HeaderMap) -> Reply {
    let user = identity(&a, &h, false).await?;
    let rows = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query(
            "SELECT contact_id, phone_card, floor(extract(epoch FROM phone_updated_at))::bigint,
                    manual_card, floor(extract(epoch FROM manual_updated_at))::bigint
             FROM contact_cards WHERE account_id=$1 AND (phone_card IS NOT NULL OR manual_card IS NOT NULL)",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?;
    Ok(Json(json!({"cards": rows.iter().map(|r| json!({
        "contact_id": r.get::<_, String>(0),
        "phone_card": r.get::<_, Option<String>>(1),
        "phone_updated_at": r.get::<_, Option<i64>>(2),
        "manual_card": r.get::<_, Option<String>>(3),
        "manual_updated_at": r.get::<_, Option<i64>>(4),
    })).collect::<Vec<_>>()})))
}
#[derive(Deserialize)]
struct Page {
    offset: Option<u32>,
    limit: Option<u16>,
    /// Only this conversation's messages.
    chat: Option<String>,
}
async fn list(State(a): State<App>, h: HeaderMap, Query(page): Query<Page>) -> Reply {
    let limit = i64::from(page.limit.unwrap_or(1000));
    if !(1..=1000).contains(&limit) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let offset = i64::from(page.offset.unwrap_or(0));
    if page.chat.as_deref().is_some_and(|c| !valid_chat(c)) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let user = identity(&a, &h, false).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let rows = db
        .query(
            "SELECT m.id,m.sender,m.body,m.contact_id,extract(epoch from m.received_at)::bigint,m.read_at IS NOT NULL,m.starred,m.chat_id,
                    (SELECT json_agg(json_build_object('id',x.id,'envelope',x.envelope))::text FROM message_reactions x WHERE x.account_id=m.account_id AND x.message_id=m.id),
                    (SELECT count(*) FROM message_attachments t WHERE t.account_id=m.account_id AND t.message_id=m.id)
             FROM archived_messages m WHERE m.account_id=$1 AND ($4::text IS NULL OR m.chat_id=$4)
             ORDER BY m.received_at DESC,m.id LIMIT $2 OFFSET $3",
            &[&user, &(limit + 1), &offset, &page.chat],
        )
        .await
        .map_err(|_| DOWN)?;
    let messages: Vec<Value> = rows
        .iter()
        .take(limit as usize)
        .map(|r| {
            let mut m = json!({"id":r.get::<_,String>(0),"sender":r.get::<_,String>(1),"body":r.get::<_,String>(2),"contact_id":r.get::<_,Option<String>>(3),"received_at":r.get::<_,i64>(4),"read":r.get::<_,bool>(5),"starred":r.get::<_,bool>(6)});
            if let Some(chat) = r.get::<_, Option<String>>(7) {
                m["chat_id"] = json!(chat);
            }
            if let Some(reactions) = r
                .get::<_, Option<String>>(8)
                .and_then(|j| serde_json::from_str::<Value>(&j).ok())
            {
                m["reactions"] = reactions;
            }
            let files: i64 = r.get(9);
            if files > 0 {
                m["attachments"] = json!(files);
            }
            m
        })
        .collect();
    Ok(Json(
        json!({"next_offset":(rows.len()>limit as usize).then_some(offset+limit),"messages":messages}),
    ))
}
/// A phone's daily check-in (pSMS Check-in on a Time of Day automation), with its upload token.
async fn checkin(State(a): State<App>, h: HeaderMap) -> Result<StatusCode, Failure> {
    let digest = bearer(&h)?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let found = db
        .execute(
            "UPDATE sessions SET checked_in_at=now() WHERE digest=$1 AND scope='upload' AND expires_at>now()",
            &[&digest],
        )
        .await
        .map_err(|_| DOWN)?;
    if found == 0 {
        return Err(DENIED);
    }
    Ok(StatusCode::NO_CONTENT)
}
/// pSMS for Mac saw a text in Messages that the phone never sent.
async fn missed(State(a): State<App>, h: HeaderMap) -> Result<StatusCode, Failure> {
    let user = identity(&a, &h, false).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    db.execute(
        "INSERT INTO capture_health(account_id,missed_at) VALUES($1,now())
         ON CONFLICT(account_id) DO UPDATE SET missed_at=now()",
        &[&user],
    )
    .await
    .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
/// Is each phone still sending: its last text and check-in, recent volume, and the last text
/// pSMS for Mac saw that no phone sent.
async fn health(State(a): State<App>, h: HeaderMap) -> Reply {
    let user = identity(&a, &h, false).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let phones = db
        .query(
            "SELECT substr(encode(digest,'hex'),1,16), floor(extract(epoch FROM last_used_at))::bigint,
                    floor(extract(epoch FROM checked_in_at))::bigint
             FROM sessions WHERE account_id=$1 AND scope='upload' AND expires_at>now()
             ORDER BY last_used_at DESC NULLS LAST LIMIT 50",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?;
    let recent: i64 = db
        .query_one(
            "SELECT count(*) FROM archived_messages WHERE account_id=$1 AND received_at > now() - interval '7 days'",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?
        .get(0);
    let missed: Option<i64> = db
        .query_opt(
            "SELECT floor(extract(epoch FROM missed_at))::bigint FROM capture_health WHERE account_id=$1",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?
        .and_then(|r| r.get(0));
    Ok(Json(json!({
        "phones": phones.iter().map(|r| json!({
            "id": r.get::<_, String>(0),
            "last_message_at": r.get::<_, Option<i64>>(1),
            "checked_in_at": r.get::<_, Option<i64>>(2),
        })).collect::<Vec<_>>(),
        "recent_messages_7d": recent,
        "missed_at": missed,
    })))
}
// Phone upload tokens for Settings. A token is identified by the start of its digest, never the token.
async fn phones(State(a): State<App>, h: HeaderMap) -> Reply {
    let user = identity(&a, &h, false).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let rows = db
        .query(
            "SELECT substr(encode(digest,'hex'),1,16),
                    floor(extract(epoch FROM expires_at - interval '1 year'))::bigint,
                    floor(extract(epoch FROM used_at))::bigint,
                    floor(extract(epoch FROM last_used_at))::bigint, scope,
                    floor(extract(epoch FROM checked_in_at))::bigint
             FROM sessions WHERE account_id=$1 AND scope IN ('upload','computer') AND expires_at>now()
             ORDER BY last_used_at DESC NULLS LAST, expires_at DESC LIMIT 50",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?;
    Ok(Json(json!({"phones": rows.iter().map(|r| json!({
        "id": r.get::<_, String>(0),
        "connected_at": r.get::<_, i64>(1),
        "first_message_at": r.get::<_, Option<i64>>(2),
        "last_message_at": r.get::<_, Option<i64>>(3),
        "kind": if r.get::<_, String>(4) == "computer" { "computer" } else { "phone" },
        "checked_in_at": r.get::<_, Option<i64>>(5),
    })).collect::<Vec<_>>()})))
}
async fn remove_phone(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let user = identity(&a, &h, false).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    match db
        .execute(
            "DELETE FROM sessions WHERE account_id=$1 AND scope IN ('upload','computer') AND substr(encode(digest,'hex'),1,16)=lower($2)",
            &[&user, &id],
        )
        .await
        .map_err(|_| DOWN)?
    {
        0 => Err(StatusCode::NOT_FOUND),
        _ => Ok(StatusCode::NO_CONTENT),
    }
}
#[derive(Deserialize)]
struct ReadIds {
    ids: Vec<Uuid>,
}
async fn mark_read(
    State(a): State<App>,
    h: HeaderMap,
    Json(read): Json<ReadIds>,
) -> Result<StatusCode, Failure> {
    if read.ids.is_empty() || read.ids.len() > 500 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let user = identity(&a, &h, false).await?;
    let ids: Vec<String> = read.ids.iter().map(Uuid::to_string).collect();
    a.pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .execute(
            "UPDATE archived_messages SET read_at=now() WHERE account_id=$1 AND id=ANY($2) AND read_at IS NULL",
            &[&user, &ids],
        )
        .await
        .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    starred: Option<bool>,
    read: Option<bool>,
}
/// Stars or marks one message read, matching the local archive API.
async fn update(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(edit): Json<Edit>,
) -> Result<StatusCode, Failure> {
    if edit.starred.is_none() && edit.read.is_none() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let user = identity(&a, &h, false).await?;
    let changed = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .execute(
            "UPDATE archived_messages SET starred=coalesce($3,starred),
                read_at=CASE WHEN $4::boolean IS NULL THEN read_at WHEN $4 THEN coalesce(read_at,now()) ELSE NULL END
             WHERE account_id=$1 AND id=$2",
            &[&user, &id.to_string(), &edit.starred, &edit.read],
        )
        .await
        .map_err(|_| DOWN)?;
    if changed == 0 {
        Err(StatusCode::NOT_FOUND)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}
async fn remove(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Failure> {
    let user = identity(&a, &h, false).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    match db
        .execute(
            "DELETE FROM archived_messages WHERE account_id=$1 AND id=$2",
            &[&user, &id.to_string()],
        )
        .await
        .map_err(|_| DOWN)?
    {
        0 => Err(StatusCode::NOT_FOUND),
        _ => Ok(StatusCode::NO_CONTENT),
    }
}
async fn ready(State(a): State<App>) -> Result<StatusCode, Failure> {
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    db.query("SELECT id FROM accounts LIMIT 0", &[])
        .await
        .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let passfile =
        std::env::var("PSMS_HOSTED_PGPASS").unwrap_or_else(|_| "/etc/psms/hosted-db/pgpass".into());
    let file = std::fs::read_to_string(passfile)?;
    let parts: Vec<_> = file.trim_end().split(':').collect();
    let [host, port, name, user, pass] = parts.as_slice() else {
        return Err("invalid hosted database configuration".into());
    };
    if *host != "127.0.0.1" {
        return Err("a loopback database tunnel is required".into());
    }
    let mut conf = Config::new();
    conf.host(*host)
        .port(port.parse()?)
        .dbname(*name)
        .user(*user)
        .password(pass)
        .connect_timeout(Duration::from_secs(4));
    let (arrivals, _) = tokio::sync::broadcast::channel(128);
    tokio::spawn(browser::listen(conf.clone(), arrivals.clone()));
    let pool = Pool::builder(Manager::new(conf, NoTls))
        .max_size(4)
        .build()?;
    let app = App {
        arrivals,
        pool,
        plans: Default::default(),
        exchange_key: std::fs::read_to_string(
            std::env::var("PSMS_HOSTED_IDENTITY_KEY")
                .unwrap_or_else(|_| "/etc/psms/hosted-db/identity-key".into()),
        )?
        .trim()
        .to_owned(),
    };
    webhooks::start(app.clone());
    let api = Router::new()
        .route("/readyz", get(ready))
        .route("/auth/exchange", post(exchange))
        .route("/auth/session", delete(logout))
        .route("/me", get(me))
        .route(
            "/auth/browser",
            post(browser::issue).delete(browser::revoke),
        )
        .route("/otp/recent", get(browser::recent))
        .route("/otp/events", get(browser::events))
        .route("/plans", get(plans::list))
        .route("/admin/plans", get(plans::admin_list))
        .route("/admin/plans/{id}", put(plans::admin_save))
        .route("/admin/accounts", get(plans::admin_accounts))
        .route("/sync/push", post(sync::push))
        .route("/sync/pull", get(sync::pull))
        .route("/team", get(teams::list).post(teams::add))
        .route("/team/{id}", delete(teams::remove))
        .route("/webhooks", get(webhooks::list).post(webhooks::add))
        .route("/webhooks/{id}", delete(webhooks::remove))
        .route("/webhooks/deliveries", get(webhooks::log))
        .route("/webhooks/deliveries/{id}/replay", post(webhooks::replay))
        .route("/admin/accounts/{id}/plan", put(plans::admin_assign))
        .route("/pair", post(pair))
        .route("/computers", post(computer))
        .route("/outbox", get(outbox::list).post(outbox::queue))
        .route("/outbox/claim", post(outbox::claim))
        .route("/outbox/{id}", get(outbox::status).delete(outbox::cancel))
        .route("/outbox/{id}/result", post(outbox::result))
        .route("/pair/seen", get(seen).post(mark_seen))
        .route("/pair/ready", post(mark_ready))
        .route("/phones", get(phones))
        .route("/checkin", post(checkin))
        .route("/health", get(health))
        .route("/health/missed", post(missed))
        .route("/phones/{id}", delete(remove_phone))
        .route("/messages", get(list).post(store))
        .route("/messages/read", post(mark_read))
        .route("/contacts/cards", get(cards))
        .route(
            "/contacts/{id}/card",
            put(put_card).layer(DefaultBodyLimit::max(512 * 1024)),
        )
        .route("/messages/{id}", delete(remove).patch(update))
        .route("/chats", get(list_chats))
        .route("/chats/{id}", put(put_chat))
        .route(
            "/messages/{id}/reactions/{reaction}",
            put(put_reaction).delete(delete_reaction),
        )
        .route(
            "/messages/{id}/attachments/{index}",
            get(get_attachment)
                .put(put_attachment)
                .layer(DefaultBodyLimit::max(ATTACHMENT_MAX + 4096)),
        )
        .route("/events", get(browser::inbox_events))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(axum::middleware::from_fn_with_state(app.clone(), throttle))
        .with_state(app);
    axum::serve(
        tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            std::env::var("PSMS_HOSTED_PORT")
                .unwrap_or_else(|_| "47833".into())
                .parse::<u16>()?,
        ))
        .await?,
        Router::new().nest("/free/v1", api),
    )
    .await?;
    Ok(())
}
