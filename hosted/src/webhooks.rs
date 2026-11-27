//! Managed webhooks (Plus): Papamo tells your service when a message arrives, retries with
//! backoff, keeps a delivery log and replays on request. Payloads carry IDs only, never
//! message contents; the receiver fetches the sealed message with its own token.
use crate::{App, DOWN, Failure, Reply, identity, plans};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::json;
use std::{net::IpAddr, time::Duration};

const ATTEMPTS: i32 = 10;

async fn allowed(a: &App, h: &HeaderMap) -> Result<i64, Failure> {
    let user = identity(a, h, false).await?;
    if plans::gates(a, user).await?.webhooks {
        Ok(user)
    } else {
        Err(StatusCode::PAYMENT_REQUIRED)
    }
}

/// Only public internet addresses: never this server, its database tunnel or a private network.
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || a >= 224)
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => public(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || (first & 0xfe00) == 0xfc00
                    || (first & 0xffc0) == 0xfe80
                    || (first & 0xff00) == 0xff00)
            }
        },
    }
}
fn valid_url(raw: &str) -> Option<url::Url> {
    let u = url::Url::parse(raw).ok()?;
    (u.scheme() == "https"
        && u.username().is_empty()
        && u.password().is_none()
        && u.fragment().is_none()
        && u.host_str().is_some()
        && raw.len() <= 500)
        .then_some(u)
}
/// Resolves once and pins the connection to that public address, so DNS cannot redirect
/// a delivery to an internal one.
async fn client_for(u: &url::Url) -> Result<reqwest::Client, String> {
    let host = u.host_str().ok_or("No host")?.to_string();
    let port = u.port_or_known_default().unwrap_or(443);
    let addr = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| "Could not resolve the host".to_string())?
        .find(|a| public(a.ip()))
        .ok_or("The host is not a public internet address")?;
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .resolve(&host, addr)
        .build()
        .map_err(|_| "Could not start the delivery".to_string())
}

pub async fn list(State(a): State<App>, h: HeaderMap) -> Reply {
    let user = identity(&a, &h, false).await?;
    let rows = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query(
            "SELECT id,url,floor(extract(epoch FROM created_at))::bigint FROM hosted_webhooks WHERE account_id=$1 ORDER BY created_at",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?;
    Ok(Json(json!({
        "available": plans::gates(&a, user).await?.webhooks,
        "webhooks": rows.iter().map(|r| json!({"id": r.get::<_, String>(0), "url": r.get::<_, String>(1), "created_at": r.get::<_, i64>(2)})).collect::<Vec<_>>(),
    })))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Add {
    url: String,
}
/// Adds a destination. The returned secret is sent as a bearer token with each delivery and is
/// shown only once.
pub async fn add(State(a): State<App>, h: HeaderMap, Json(add): Json<Add>) -> Reply {
    let user = allowed(&a, &h).await?;
    let u = valid_url(add.url.trim()).ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let count: i64 = db
        .query_one(
            "SELECT count(*) FROM hosted_webhooks WHERE account_id=$1",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?
        .get(0);
    if count >= 10 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let added = db
        .execute(
            "INSERT INTO hosted_webhooks(id,account_id,url,secret) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING",
            &[&id, &user, &u.as_str(), &secret],
        )
        .await
        .map_err(|_| DOWN)?;
    if added == 0 {
        return Err(StatusCode::CONFLICT);
    }
    Ok(Json(json!({"id": id, "url": u.as_str(), "secret": secret})))
}
pub async fn remove(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    let user = identity(&a, &h, false).await?;
    let gone = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .execute(
            "DELETE FROM hosted_webhooks WHERE id=$1 AND account_id=$2",
            &[&id, &user],
        )
        .await
        .map_err(|_| DOWN)?;
    if gone == 0 {
        Err(StatusCode::NOT_FOUND)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}
/// The latest 100 deliveries, newest first.
pub async fn log(State(a): State<App>, h: HeaderMap) -> Reply {
    let user = identity(&a, &h, false).await?;
    let rows = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query(
            "SELECT d.id, w.url, d.message_id, d.event, d.status, d.attempts, d.last_status, d.last_error,
               floor(extract(epoch FROM d.created_at))::bigint, floor(extract(epoch FROM d.delivered_at))::bigint
             FROM hosted_webhook_deliveries d JOIN hosted_webhooks w ON w.id=d.webhook_id
             WHERE d.account_id=$1 ORDER BY d.id DESC LIMIT 100",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?;
    Ok(Json(json!({"deliveries": rows.iter().map(|r| json!({
        "id": r.get::<_, i64>(0), "url": r.get::<_, String>(1), "message_id": r.get::<_, String>(2),
        "event": r.get::<_, String>(3), "status": r.get::<_, String>(4), "attempts": r.get::<_, i32>(5),
        "last_status": r.get::<_, Option<i32>>(6), "last_error": r.get::<_, Option<String>>(7),
        "created_at": r.get::<_, i64>(8), "delivered_at": r.get::<_, Option<i64>>(9),
    })).collect::<Vec<_>>()})))
}
pub async fn replay(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, Failure> {
    let user = allowed(&a, &h).await?;
    let changed = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .execute(
            "UPDATE hosted_webhook_deliveries SET status='pending', attempts=0, due=now(), last_error=NULL
             WHERE id=$1 AND account_id=$2",
            &[&id, &user],
        )
        .await
        .map_err(|_| DOWN)?;
    if changed == 0 {
        Err(StatusCode::NOT_FOUND)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

/// Delivers due events. Every region runs this; `SKIP LOCKED` and a short lease keep each
/// delivery with one worker at a time.
pub fn start(a: App) {
    tokio::spawn(async move {
        loop {
            let worked = deliver_some(&a).await.unwrap_or(false);
            if !worked {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    });
}
async fn deliver_some(a: &App) -> Result<bool, ()> {
    let mut db = a.pool.get().await.map_err(|_| ())?;
    let tx = db.transaction().await.map_err(|_| ())?;
    let rows = tx
        .query(
            "SELECT d.id, d.message_id, d.event, d.attempts, w.url, w.secret FROM hosted_webhook_deliveries d
             JOIN hosted_webhooks w ON w.id=d.webhook_id
             WHERE d.status='pending' AND d.due<=now() ORDER BY d.due LIMIT 5 FOR UPDATE OF d SKIP LOCKED",
            &[],
        )
        .await
        .map_err(|_| ())?;
    for r in &rows {
        tx.execute(
            "UPDATE hosted_webhook_deliveries SET due=now()+interval '1 minute' WHERE id=$1",
            &[&r.get::<_, i64>(0)],
        )
        .await
        .map_err(|_| ())?;
    }
    tx.commit().await.map_err(|_| ())?;
    drop(db);
    for r in &rows {
        let (id, message, event, attempts, url, secret): (
            i64,
            String,
            String,
            i32,
            String,
            String,
        ) = (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5));
        let outcome = match valid_url(&url) {
            None => Err((None, "Invalid destination".to_string())),
            Some(u) => match client_for(&u).await {
                Err(e) => Err((None, e)),
                Ok(client) => match client
                    .post(u)
                    .bearer_auth(&secret)
                    .json(&json!({"delivery_id": id, "event": event, "message_id": message}))
                    .send()
                    .await
                {
                    Ok(res) if res.status().is_success() => Ok(res.status().as_u16() as i32),
                    Ok(res) => Err((
                        Some(res.status().as_u16() as i32),
                        format!("HTTP {}", res.status().as_u16()),
                    )),
                    Err(_) => Err((None, "Could not connect".to_string())),
                },
            },
        };
        let db = a.pool.get().await.map_err(|_| ())?;
        let _ = match outcome {
            Ok(code) => {
                db.execute(
                    "UPDATE hosted_webhook_deliveries SET status='delivered', delivered_at=now(), attempts=attempts+1, last_status=$2, last_error=NULL WHERE id=$1",
                    &[&id, &code],
                )
                .await
            }
            Err((code, error)) => {
                let next = attempts + 1;
                db.execute(
                    "UPDATE hosted_webhook_deliveries SET attempts=$2, last_status=$3, last_error=$4,
                       status=CASE WHEN $2::int >= $5::int THEN 'failed' ELSE 'pending' END,
                       due=now() + make_interval(secs => least(3600::float8, 10 * power(2::float8, $2::float8)))
                     WHERE id=$1",
                    &[&id, &next, &code, &error, &ATTEMPTS],
                )
                .await
            }
        };
    }
    Ok(!rows.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_public_https_destinations() {
        for no in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "::ffff:127.0.0.1",
            "0.0.0.0",
        ] {
            assert!(!public(no.parse().unwrap()), "{no}");
        }
        for ok in ["8.8.8.8", "1.1.1.1", "2606:4700::1111"] {
            assert!(public(ok.parse().unwrap()), "{ok}");
        }
        assert!(valid_url("https://example.com/hook").is_some());
        assert!(valid_url("http://example.com/hook").is_none());
        assert!(valid_url("https://u:p@example.com/").is_none());
    }
}
