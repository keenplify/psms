//! Team inboxes (Plus): an owner adds other Papamo accounts as members. Members open the
//! owner's inbox with the `x-psms-inbox` header and read it with the owner's recovery words.
use crate::{App, DOWN, Failure, Reply, identity, plans};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::json;

/// The current account's own identity, never a shared inbox.
async fn me(a: &App, h: &HeaderMap) -> Result<i64, Failure> {
    let mut own = h.clone();
    own.remove("x-psms-inbox");
    identity(a, &own, false).await
}
/// Members of my inbox, and the inboxes shared with me.
pub async fn list(State(a): State<App>, h: HeaderMap) -> Reply {
    let me = me(&a, &h).await?;
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let members = db
        .query(
            "SELECT a.id, coalesce(a.display_name,a.username) FROM account_members m JOIN accounts a ON a.id=m.member_id
             WHERE m.owner_id=$1 ORDER BY m.added_at",
            &[&me],
        )
        .await
        .map_err(|_| DOWN)?;
    let shared = db
        .query(
            "SELECT a.id, coalesce(a.display_name,a.username), q.teams FROM account_members m
             JOIN accounts a ON a.id=m.owner_id JOIN archive_quota q ON q.id=a.id
             WHERE m.member_id=$1 ORDER BY m.added_at",
            &[&me],
        )
        .await
        .map_err(|_| DOWN)?;
    Ok(Json(json!({
        "account_id": me,
        "members": members.iter().map(|r| json!({"id": r.get::<_, i64>(0), "name": r.get::<_, String>(1)})).collect::<Vec<_>>(),
        "shared_with_me": shared.iter().filter(|r| r.get::<_, bool>(2)).map(|r| json!({"id": r.get::<_, i64>(0), "name": r.get::<_, String>(1)})).collect::<Vec<_>>(),
    })))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invite {
    /// The member's account number, shown in their Account menu.
    account_id: i64,
}
pub async fn add(
    State(a): State<App>,
    h: HeaderMap,
    Json(i): Json<Invite>,
) -> Result<StatusCode, Failure> {
    let me = me(&a, &h).await?;
    if !plans::gates(&a, me).await?.teams {
        return Err(StatusCode::PAYMENT_REQUIRED);
    }
    if i.account_id == me {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let db = a.pool.get().await.map_err(|_| DOWN)?;
    let count: i64 = db
        .query_one(
            "SELECT count(*) FROM account_members WHERE owner_id=$1",
            &[&me],
        )
        .await
        .map_err(|_| DOWN)?
        .get(0);
    if count >= 50 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    match db
        .execute(
            "INSERT INTO account_members(owner_id,member_id) SELECT $1,id FROM accounts WHERE id=$2 ON CONFLICT DO NOTHING",
            &[&me, &i.account_id],
        )
        .await
        .map_err(|_| DOWN)?
    {
        0 if db
            .query_opt("SELECT 1 FROM accounts WHERE id=$1", &[&i.account_id])
            .await
            .map_err(|_| DOWN)?
            .is_none() =>
        {
            Err(StatusCode::NOT_FOUND)
        }
        _ => Ok(StatusCode::NO_CONTENT),
    }
}
/// Removes a member from my inbox, or leaves an inbox shared with me.
pub async fn remove(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, Failure> {
    let me = me(&a, &h).await?;
    a.pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .execute(
            "DELETE FROM account_members WHERE (owner_id=$1 AND member_id=$2) OR (owner_id=$2 AND member_id=$1)",
            &[&me, &id],
        )
        .await
        .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
