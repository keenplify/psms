use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post},
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

type Failure = StatusCode;
type Reply = Result<Json<Value>, Failure>;
const DOWN: Failure = StatusCode::SERVICE_UNAVAILABLE;
const DENIED: Failure = StatusCode::UNAUTHORIZED;
const LIMIT: i64 = 1000;

#[derive(Clone)]
struct App {
    pool: Pool,
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
}

fn bearer(headers: &HeaderMap) -> Result<Vec<u8>, Failure> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| v.len() == 64)
        .ok_or(DENIED)?;
    Ok(Sha256::digest(token.as_bytes()).to_vec())
}
async fn identity(app: &App, h: &HeaderMap, upload: bool) -> Result<i64, Failure> {
    let db = app.pool.get().await.map_err(|_| DOWN)?;
    db.query_opt("SELECT account_id FROM sessions WHERE digest=$1 AND expires_at>now() AND (scope='owner' OR $2)",&[&bearer(h)?,&upload]).await.map_err(|_|DOWN)?.map(|r|r.get(0)).ok_or(DENIED)
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
    let row=db.query_one("SELECT coalesce(display_name,username),(SELECT count(*) FROM archived_messages WHERE account_id=$1) FROM accounts WHERE id=$1",&[&id]).await.map_err(|_|DOWN)?;
    Ok(Json(
        json!({"username":row.get::<_,String>(0),"used":row.get::<_,i64>(1),"limit":LIMIT}),
    ))
}
async fn pair(State(a): State<App>, h: HeaderMap) -> Reply {
    let id = identity(&a, &h, false).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let (token, digest) = new_token();
    db.execute(
        "INSERT INTO sessions VALUES($1,$2,now()+interval '1 year','upload')",
        &[&digest, &id],
    )
    .await
    .map_err(|_| DOWN)?;
    Ok(Json(json!({"token":token})))
}
async fn store(
    State(a): State<App>,
    h: HeaderMap,
    Json(m): Json<Message>,
) -> Result<StatusCode, Failure> {
    let user = identity(&a, &h, true).await?;
    if !encryption::accepts(&m.sender, &m.body)
        || m.contact_id
            .as_ref()
            .is_some_and(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let id = m.id.to_string();
    let contact = m.contact_id.map(|s| s.to_ascii_lowercase());
    let mut db = a.pool.get().await.map_err(|_| DOWN)?;
    let tx = db.transaction().await.map_err(|_| DOWN)?;
    tx.query_one("SELECT id FROM accounts WHERE id=$1 FOR UPDATE", &[&user])
        .await
        .map_err(|_| DOWN)?;
    if let Some(row)=tx.query_opt("SELECT sender=$3 AND body=$4 AND contact_id IS NOT DISTINCT FROM $5 FROM archived_messages WHERE account_id=$1 AND id=$2",&[&user,&id,&m.sender,&m.body,&contact]).await.map_err(|_|DOWN)?{
 return if row.get::<_,bool>(0){Ok(StatusCode::OK)}else{Err(StatusCode::CONFLICT)}
 }
    let count: i64 = tx
        .query_one(
            "SELECT count(*) FROM archived_messages WHERE account_id=$1",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?
        .get(0);
    if count >= LIMIT {
        return Err(StatusCode::INSUFFICIENT_STORAGE);
    }
    tx.execute("INSERT INTO archived_messages(account_id,id,sender,body,contact_id) VALUES($1,$2,$3,$4,$5)",&[&user,&id,&m.sender,&m.body,&contact]).await.map_err(|_|DOWN)?;
    tx.commit().await.map_err(|_| DOWN)?;
    Ok(StatusCode::CREATED)
}
async fn list(State(a): State<App>, h: HeaderMap) -> Reply {
    let user = identity(&a, &h, false).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let rows=db.query("SELECT id,sender,body,contact_id,extract(epoch from received_at)::bigint FROM archived_messages WHERE account_id=$1 ORDER BY received_at DESC,id LIMIT 1000",&[&user]).await.map_err(|_|DOWN)?;
    Ok(Json(
        json!({"messages":rows.iter().map(|r|json!({"id":r.get::<_,String>(0),"sender":r.get::<_,String>(1),"body":r.get::<_,String>(2),"contact_id":r.get::<_,Option<String>>(3),"received_at":r.get::<_,i64>(4)})).collect::<Vec<_>>()}),
    ))
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
    let file = std::fs::read_to_string("/etc/psms/hosted-db/pgpass")?;
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
    let pool = Pool::builder(Manager::new(conf, NoTls))
        .max_size(4)
        .build()?;
    let app = App {
        pool,
        exchange_key: std::fs::read_to_string("/etc/psms/hosted-db/identity-key")?
            .trim()
            .to_owned(),
    };
    let api = Router::new()
        .route("/readyz", get(ready))
        .route("/auth/exchange", post(exchange))
        .route("/auth/session", delete(logout))
        .route("/me", get(me))
        .route("/pair", post(pair))
        .route("/messages", get(list).post(store))
        .route("/messages/{id}", delete(remove))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(app);
    axum::serve(
        tokio::net::TcpListener::bind("127.0.0.1:47833").await?,
        Router::new().nest("/free/v1", api),
    )
    .await?;
    Ok(())
}
