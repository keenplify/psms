//! Plans: limits, feature gates and pricing details, edited by admins from the inbox. Public
//! plan details are cached in memory and by HTTP caches; saving a plan clears the cache.
use crate::{App, DOWN, Failure, identity};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const FRESH: Duration = Duration::from_secs(300);
const COLUMNS: &str = "id,name,message_limit,api_calls_per_30_min,monthly_price_cents,available,webhooks,price_label,perks,max_phones,replies,autofill";

#[derive(Default)]
pub struct Cache(std::sync::Mutex<Option<(Instant, Value)>>);
impl Cache {
    fn get(&self) -> Option<Value> {
        let held = self.0.lock().ok()?;
        held.as_ref()
            .filter(|(at, _)| at.elapsed() < FRESH)
            .map(|(_, v)| v.clone())
    }
    fn put(&self, value: &Value) {
        if let Ok(mut held) = self.0.lock() {
            *held = Some((Instant::now(), value.clone()));
        }
    }
    fn clear(&self) {
        if let Ok(mut held) = self.0.lock() {
            *held = None;
        }
    }
}

fn row(r: &tokio_postgres::Row) -> Value {
    json!({
        "id": r.get::<_, String>(0), "name": r.get::<_, String>(1),
        "message_limit": r.get::<_, Option<i64>>(2), "api_calls_per_30_min": r.get::<_, Option<i32>>(3),
        "monthly_price_cents": r.get::<_, i32>(4), "available": r.get::<_, bool>(5),
        "webhooks": r.get::<_, bool>(6), "price_label": r.get::<_, Option<String>>(7),
        "perks": r.get::<_, Vec<String>>(8), "max_phones": r.get::<_, Option<i32>>(9),
        "replies": r.get::<_, bool>(10), "autofill": r.get::<_, bool>(11),
    })
}
async fn load(a: &App) -> Result<Value, Failure> {
    let rows = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query(
            &format!("SELECT {COLUMNS} FROM plans ORDER BY sort, id"),
            &[],
        )
        .await
        .map_err(|_| DOWN)?;
    Ok(json!({"plans": rows.iter().map(row).collect::<Vec<_>>()}))
}

/// Public plan details for pricing pages. Cached for five minutes here and by browsers and CDNs.
pub async fn list(State(a): State<App>) -> Result<Response, Failure> {
    let value = match a.plans.get() {
        Some(v) => v,
        None => {
            let v = load(&a).await?;
            a.plans.put(&v);
            v
        }
    };
    Ok((
        [(
            header::CACHE_CONTROL,
            "public, max-age=300, stale-while-revalidate=600",
        )],
        Json(value),
    )
        .into_response())
}

/// The signed-in account's effective limits and gates. Admins are unlimited.
pub struct Gates {
    pub max_phones: Option<i32>,
    pub replies: bool,
    pub autofill: bool,
    pub admin: bool,
}
pub async fn gates(a: &App, user: i64) -> Result<Gates, Failure> {
    let r = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query_one(
            "SELECT max_phones, replies, autofill, admin FROM archive_quota WHERE id=$1",
            &[&user],
        )
        .await
        .map_err(|_| DOWN)?;
    Ok(Gates {
        max_phones: r.get(0),
        replies: r.get(1),
        autofill: r.get(2),
        admin: r.get(3),
    })
}
/// Refuses a feature the plan does not include, with a message an inbox can show.
pub fn upgrade(feature: &str) -> Response {
    (
        StatusCode::PAYMENT_REQUIRED,
        Json(json!({"error": format!("{feature} is not included in your plan. Upgrade at papamo.dev/sms#plans."), "upgrade": true})),
    )
        .into_response()
}

async fn admin(a: &App, h: &HeaderMap) -> Result<(), Failure> {
    let user = identity(a, h, false).await?;
    if gates(a, user).await?.admin {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}
/// Every plan, fresh from the database, for the admin editor.
pub async fn admin_list(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, Failure> {
    admin(&a, &h).await?;
    Ok(Json(load(&a).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    name: String,
    message_limit: Option<i64>,
    api_calls_per_30_min: Option<i32>,
    monthly_price_cents: i32,
    available: bool,
    webhooks: bool,
    price_label: Option<String>,
    perks: Vec<String>,
    max_phones: Option<i32>,
    replies: bool,
    autofill: bool,
}
pub async fn admin_save(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(e): Json<Edit>,
) -> Result<Json<Value>, Failure> {
    admin(&a, &h).await?;
    let label = e
        .price_label
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());
    let perks: Vec<String> = e
        .perks
        .iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if e.name.trim().is_empty()
        || e.name.len() > 40
        || label.as_ref().is_some_and(|l| l.len() > 40)
        || perks.len() > 12
        || perks.iter().any(|p| p.len() > 160)
        || !(0..=10_000_000).contains(&e.monthly_price_cents)
        || e.message_limit.is_some_and(|v| v < 1)
        || e.api_calls_per_30_min.is_some_and(|v| v < 1)
        || e.max_phones.is_some_and(|v| v < 1)
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let r = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query_opt(
            &format!(
                "UPDATE plans SET name=$2, message_limit=$3, api_calls_per_30_min=$4, monthly_price_cents=$5,
                   available=$6, webhooks=$7, price_label=$8, perks=$9, max_phones=$10, replies=$11, autofill=$12
                 WHERE id=$1 RETURNING {COLUMNS}"
            ),
            &[
                &id, &e.name.trim(), &e.message_limit, &e.api_calls_per_30_min, &e.monthly_price_cents,
                &e.available, &e.webhooks, &label, &perks, &e.max_phones, &e.replies, &e.autofill,
            ],
        )
        .await
        .map_err(|_| DOWN)?
        .ok_or(StatusCode::NOT_FOUND)?;
    a.plans.clear();
    Ok(Json(row(&r)))
}

#[derive(Deserialize)]
pub struct Search {
    q: Option<String>,
}
/// Accounts for the admin editor, newest first, matched by name or account ID.
pub async fn admin_accounts(
    State(a): State<App>,
    h: HeaderMap,
    axum::extract::Query(s): axum::extract::Query<Search>,
) -> Result<Json<Value>, Failure> {
    admin(&a, &h).await?;
    let needle = s.q.unwrap_or_default().trim().to_lowercase();
    if needle.len() > 80 {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let pattern = format!(
        "%{}%",
        needle
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let rows = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query(
            "SELECT a.id, coalesce(a.display_name, a.username), a.plan,
               (SELECT count(*) FROM archived_messages m WHERE m.account_id=a.id), q.admin
             FROM accounts a JOIN archive_quota q USING(id)
             WHERE $1 = '%%' OR lower(coalesce(a.display_name,'')) LIKE $1 OR lower(a.username) LIKE $1 OR a.id::text = $2
             ORDER BY a.id DESC LIMIT 50",
            &[&pattern, &needle],
        )
        .await
        .map_err(|_| DOWN)?;
    Ok(Json(json!({"accounts": rows.iter().map(|r| json!({
        "id": r.get::<_, i64>(0), "name": r.get::<_, String>(1), "plan": r.get::<_, String>(2),
        "messages": r.get::<_, i64>(3), "admin": r.get::<_, bool>(4),
    })).collect::<Vec<_>>()})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assign {
    plan: String,
}
/// Moves an account to a plan by hand, such as after a payment or for a trial.
pub async fn admin_assign(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(p): Json<Assign>,
) -> Result<StatusCode, Failure> {
    admin(&a, &h).await?;
    let changed = a
        .pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .execute(
            "UPDATE accounts SET plan=$2 WHERE id=$1 AND EXISTS (SELECT 1 FROM plans WHERE id=$2)",
            &[&id, &p.plan],
        )
        .await
        .map_err(|_| DOWN)?;
    if changed == 0 {
        Err(StatusCode::NOT_FOUND)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}
