//! Connects this self-hosted archive to a papamo.dev Plus account. The service reaches out to
//! papamo.dev, so nothing here needs a tunnel or public address:
//! - phones send to papamo.dev, and new messages are pulled down here (webhooks fire as usual);
//! - this archive's messages are pushed up as a sealed, off-site backup that also feeds the web
//!   inbox, replies from anywhere, team inboxes and managed webhooks;
//! - restore pulls everything back down, for example onto a new computer.
//!
//! Messages and contact cards stay sealed with the archive key in both directions.
use crate::Archive;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

pub const HOSTED: &str = "https://na.sms.papamo.dev/free/v1";
pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS cloud_link(id INTEGER PRIMARY KEY CHECK(id=1), base TEXT NOT NULL, token TEXT NOT NULL,
    after INTEGER NOT NULL DEFAULT 0, after_id TEXT NOT NULL DEFAULT '', last_sync INTEGER, error TEXT,
    pushed INTEGER NOT NULL DEFAULT 0, pulled INTEGER NOT NULL DEFAULT 0, cards_at INTEGER NOT NULL DEFAULT 0);
  CREATE TABLE IF NOT EXISTS cloud_synced(message_id TEXT PRIMARY KEY NOT NULL)";

type Reply = Result<Json<Value>, StatusCode>;

fn owner(archive: &Archive, headers: &HeaderMap) -> Result<(), StatusCode> {
    if !archive.is_owner(headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if archive.remote.is_some() {
        return Err(StatusCode::NOT_IMPLEMENTED);
    }
    Ok(())
}
async fn status(State(archive): State<Archive>, headers: HeaderMap) -> Reply {
    owner(&archive, &headers)?;
    let row = archive
        .query(|db| {
            db.query_row(
                "SELECT base,last_sync,error,pushed,pulled,(SELECT count(*) FROM messages WHERE id NOT IN (SELECT message_id FROM cloud_synced)) FROM cloud_link WHERE id=1",
                [],
                |r| {
                    Ok(json!({"connected": true, "base": r.get::<_, String>(0)?, "last_sync": r.get::<_, Option<i64>>(1)?,
                        "error": r.get::<_, Option<String>>(2)?, "pushed": r.get::<_, i64>(3)?, "pulled": r.get::<_, i64>(4)?,
                        "waiting": r.get::<_, i64>(5)?}))
                },
            )
            .optional()
        })
        .await?;
    Ok(Json(row.unwrap_or_else(|| json!({"connected": false}))))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    token: String,
    base: Option<String>,
}
fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}
/// Connects with a papamo.dev computer token, after checking the plan includes it.
async fn connect(
    State(archive): State<Archive>,
    headers: HeaderMap,
    Json(link): Json<Link>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let fail = |code: StatusCode, msg: &str| (code, Json(json!({"error": msg})));
    owner(&archive, &headers).map_err(|c| fail(c, "Not allowed."))?;
    let base = link
        .base
        .unwrap_or_else(|| HOSTED.into())
        .trim_end_matches('/')
        .to_string();
    let loopback = base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:");
    if !(base.starts_with("https://") || loopback) || link.token.trim().len() != 64 {
        return Err(fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Use a papamo.dev computer token.",
        ));
    }
    let token = link.token.trim().to_string();
    let check = http()
        .get(format!("{base}/sync/pull?limit=1&after={}", i64::MAX))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|_| fail(StatusCode::BAD_GATEWAY, "Could not reach papamo.dev."))?;
    match check.status().as_u16() {
        200 => {}
        402 => {
            return Err(fail(
                StatusCode::PAYMENT_REQUIRED,
                "Connecting a self-hosted archive needs a Papamo Plus plan.",
            ));
        }
        401 | 403 => {
            return Err(fail(
                StatusCode::UNAUTHORIZED,
                "papamo.dev did not accept that token.",
            ));
        }
        _ => {
            return Err(fail(
                StatusCode::BAD_GATEWAY,
                "papamo.dev could not be reached.",
            ));
        }
    }
    archive
        .query(move |db| {
            db.execute(
                "INSERT INTO cloud_link(id,base,token) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET base=excluded.base, token=excluded.token, error=NULL",
                params![base, token],
            )
        })
        .await
        .map_err(|c| fail(c, "Could not save the connection."))?;
    archive.arrivals.send_replace(());
    Ok(StatusCode::NO_CONTENT)
}
async fn disconnect(
    State(archive): State<Archive>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    owner(&archive, &headers)?;
    archive
        .query(|db| db.execute_batch("DELETE FROM cloud_link; DELETE FROM cloud_synced;"))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
/// Pulls the whole papamo.dev copy down again. Messages already here are kept as they are.
async fn restore(
    State(archive): State<Archive>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    owner(&archive, &headers)?;
    let changed = archive
        .query(|db| {
            db.execute(
                "UPDATE cloud_link SET after=0, after_id='', cards_at=0 WHERE id=1",
                [],
            )
        })
        .await?;
    if changed == 0 {
        return Err(StatusCode::CONFLICT);
    }
    archive.arrivals.send_replace(());
    Ok(StatusCode::ACCEPTED)
}
pub fn routes(prefix: &str, archive: Archive) -> Router<Archive> {
    Router::new()
        .route(
            &format!("{prefix}/cloud"),
            get(status).put(connect).delete(disconnect),
        )
        .route(&format!("{prefix}/cloud/restore"), post(restore))
        .with_state(archive)
}

struct Config {
    base: String,
    token: String,
    after: i64,
    after_id: String,
    cards_at: i64,
}
/// Syncs every 20 seconds, and soon after anything arrives here.
pub fn start(archive: Archive) {
    if archive.remote.is_some() {
        return;
    }
    tokio::spawn(async move {
        let mut changes = archive.arrivals.subscribe();
        let client = http();
        loop {
            let result = round(&archive, &client).await;
            let more = matches!(result, Ok(true));
            let error = result.err();
            let _ = archive
                .query(move |db| {
                    db.execute(
                        "UPDATE cloud_link SET last_sync=unixepoch(), error=?1 WHERE id=1",
                        [error],
                    )
                })
                .await;
            if more {
                continue;
            }
            let _ = tokio::time::timeout(Duration::from_secs(20), changes.changed()).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}
/// One push and pull. Returns true when there is more to do right away.
async fn round(archive: &Archive, client: &reqwest::Client) -> Result<bool, String> {
    let Some(cfg) = archive
        .query(|db| {
            db.query_row(
                "SELECT base,token,after,after_id,cards_at FROM cloud_link WHERE id=1",
                [],
                |r| {
                    Ok(Config {
                        base: r.get(0)?,
                        token: r.get(1)?,
                        after: r.get(2)?,
                        after_id: r.get(3)?,
                        cards_at: r.get(4)?,
                    })
                },
            )
            .optional()
        })
        .await
        .map_err(|_| "Could not read the archive.".to_string())?
    else {
        return Ok(false);
    };
    let explain = |status: reqwest::StatusCode| match status.as_u16() {
        402 => "Your Papamo plan no longer includes connecting a self-hosted archive.".to_string(),
        401 | 403 => "papamo.dev no longer accepts this archive. Connect it again.".to_string(),
        429 => "papamo.dev asked to slow down; syncing resumes shortly.".to_string(),
        code => format!("papamo.dev returned HTTP {code}."),
    };
    // Push messages papamo.dev does not have yet.
    let batch = archive
        .query(|db| {
            let mut stmt = db.prepare(
                "SELECT m.id,m.sender,m.body,c.contact_id,m.archived_at FROM messages m
                 LEFT JOIN message_contacts c ON c.message_id=m.id
                 WHERE m.id NOT IN (SELECT message_id FROM cloud_synced) ORDER BY m.rowid LIMIT 200",
            )?;
            stmt.query_map([], |r| {
                Ok(json!({"id": r.get::<_, String>(0)?, "sender": r.get::<_, String>(1)?, "body": r.get::<_, String>(2)?,
                    "contact_id": r.get::<_, Option<String>>(3)?, "archived_at": r.get::<_, i64>(4)?}))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .await
        .map_err(|_| "Could not read the archive.".to_string())?;
    let pushed_full = batch.len() == 200;
    if !batch.is_empty() {
        let res = client
            .post(format!("{}/sync/push", cfg.base))
            .bearer_auth(&cfg.token)
            .json(&json!({"messages": batch}))
            .send()
            .await
            .map_err(|_| "Could not reach papamo.dev.".to_string())?;
        if !res.status().is_success() {
            return Err(explain(res.status()));
        }
        let ids: Vec<String> = batch
            .iter()
            .filter_map(|m| m["id"].as_str().map(String::from))
            .collect();
        archive
            .query(move |db| {
                let tx = db.unchecked_transaction()?;
                for id in &ids {
                    tx.execute(
                        "INSERT OR IGNORE INTO cloud_synced(message_id) VALUES(?1)",
                        [id],
                    )?;
                }
                tx.execute(
                    "UPDATE cloud_link SET pushed=pushed+?1 WHERE id=1",
                    [ids.len() as i64],
                )?;
                tx.commit()
            })
            .await
            .map_err(|_| "Could not update the archive.".to_string())?;
    }
    // Pull what phones and inboxes sent to papamo.dev.
    let res = client
        .get(format!("{}/sync/pull", cfg.base))
        .query(&[
            ("after", cfg.after.to_string()),
            ("after_id", cfg.after_id.clone()),
            ("limit", "200".into()),
        ])
        .bearer_auth(&cfg.token)
        .send()
        .await
        .map_err(|_| "Could not reach papamo.dev.".to_string())?;
    if !res.status().is_success() {
        return Err(explain(res.status()));
    }
    let page: Value = res
        .json()
        .await
        .map_err(|_| "Unexpected reply from papamo.dev.".to_string())?;
    let more = page["more"] == true;
    let rows = page["messages"].as_array().cloned().unwrap_or_default();
    let next = page["next"].clone();
    let added = archive
        .query(move |db| {
            let tx = db.unchecked_transaction()?;
            let mut added = 0i64;
            for m in &rows {
                let (Some(id), Some(sender), Some(body)) = (m["id"].as_str(), m["sender"].as_str(), m["body"].as_str()) else {
                    continue;
                };
                if !crate::encryption::accepts(sender, body) {
                    continue;
                }
                let inserted = tx.execute(
                    "INSERT INTO messages(id,sender,body,archived_at) SELECT ?1,?2,?3,?4
                     WHERE NOT EXISTS (SELECT 1 FROM deleted_messages WHERE id=?1) ON CONFLICT(id) DO NOTHING",
                    params![id, sender, body, m["archived_at"].as_i64().unwrap_or_default()],
                )?;
                if inserted == 1 {
                    added += 1;
                    if let Some(contact) = m["contact_id"].as_str().filter(|c| c.len() == 64) {
                        tx.execute("INSERT INTO contacts(id) VALUES(?1) ON CONFLICT(id) DO UPDATE SET last_seen_at=max(last_seen_at,unixepoch())", [contact])?;
                        tx.execute("INSERT OR IGNORE INTO message_contacts(message_id,contact_id) VALUES(?1,?2)", params![id, contact])?;
                    }
                }
                tx.execute("INSERT OR IGNORE INTO cloud_synced(message_id) VALUES(?1)", [id])?;
            }
            if let (Some(after), Some(after_id)) = (next["after"].as_i64(), next["after_id"].as_str()) {
                tx.execute("UPDATE cloud_link SET after=?1, after_id=?2 WHERE id=1", params![after, after_id])?;
            }
            tx.execute("UPDATE cloud_link SET pulled=pulled+?1 WHERE id=1", [added])?;
            tx.commit()?;
            Ok(added)
        })
        .await
        .map_err(|_| "Could not save pulled messages.".to_string())?;
    if added > 0 {
        archive.arrivals.send_replace(());
    }
    sync_cards(archive, client, &cfg).await?;
    Ok(more || pushed_full)
}

/// Contact cards both ways: newer copies win on each side, edits made here become edits there.
async fn sync_cards(
    archive: &Archive,
    client: &reqwest::Client,
    cfg: &Config,
) -> Result<(), String> {
    let res = client
        .get(format!("{}/contacts/cards", cfg.base))
        .bearer_auth(&cfg.token)
        .send()
        .await
        .map_err(|_| "Could not reach papamo.dev.".to_string())?;
    if !res.status().is_success() {
        return Ok(());
    }
    let remote: Value = res
        .json()
        .await
        .map_err(|_| "Unexpected reply from papamo.dev.".to_string())?;
    let cards = remote["cards"].as_array().cloned().unwrap_or_default();
    let since = cfg.cards_at;
    let outgoing = archive
        .query(move |db| {
            let tx = db.unchecked_transaction()?;
            for c in &cards {
                let Some(id) = c["contact_id"].as_str().filter(|i| i.len() == 64) else { continue };
                for (card, at, col, col_at) in [("phone_card", "phone_updated_at", "phone_card", "phone_updated_at"), ("manual_card", "manual_updated_at", "manual_card", "manual_updated_at")] {
                    let Some(stamp) = c[at].as_i64() else { continue };
                    tx.execute(
                        &format!("INSERT INTO contact_cards(contact_id,{col},{col_at}) VALUES(?1,?2,?3)
                          ON CONFLICT(contact_id) DO UPDATE SET {col}=excluded.{col}, {col_at}=excluded.{col_at}
                          WHERE coalesce(contact_cards.{col_at},0) < excluded.{col_at}"),
                        params![id, c[card].as_str(), stamp],
                    )?;
                }
            }
            let mut stmt = db.prepare("SELECT contact_id, manual_card FROM contact_cards WHERE manual_updated_at > ?1")?;
            let mine = stmt
                .query_map([since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            tx.commit()?;
            Ok(mine)
        })
        .await
        .map_err(|_| "Could not save contact cards.".to_string())?;
    let mut ok = true;
    for (id, envelope) in outgoing {
        ok &= client
            .put(format!("{}/contacts/{id}/card", cfg.base))
            .bearer_auth(&cfg.token)
            .json(&json!({"envelope": envelope}))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
    }
    if ok {
        let _ = archive
            .query(|db| db.execute("UPDATE cloud_link SET cards_at=unixepoch() WHERE id=1", []))
            .await;
    }
    Ok(())
}
