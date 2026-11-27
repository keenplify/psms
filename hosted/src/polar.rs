//! Polar bills the owner of a personal archive. Team inbox selection never changes the payer.
use crate::{App, DOWN, Failure, bearer};
use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use reqwest::{Client, Method};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

const RETURN: &str = "https://www.papamo.dev/sms/inbox";

struct Polar {
    client: Client,
    base: &'static str,
    token: String,
    product: Uuid,
    hook: standardwebhooks::Webhook,
}

impl Polar {
    fn load() -> Result<Self, Failure> {
        let setting = |name| {
            std::env::var(name)
                .ok()
                .filter(|s| !s.trim().is_empty())
                .ok_or(DOWN)
        };
        Ok(Self {
            client: Client::builder()
                .user_agent("psms-hosted/1.0")
                .timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| DOWN)?,
            base: match std::env::var("PSMS_POLAR_ENV").as_deref() {
                Ok("sandbox") => "https://sandbox-api.polar.sh/v1",
                Err(_) | Ok("production") => "https://api.polar.sh/v1",
                _ => return Err(DOWN),
            },
            token: setting("PSMS_POLAR_TOKEN")?,
            product: setting("PSMS_POLAR_PRODUCT")?.parse().map_err(|_| DOWN)?,
            hook: standardwebhooks::Webhook::new(&setting("PSMS_POLAR_WEBHOOK_SECRET")?)
                .map_err(|_| DOWN)?,
        })
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, Failure> {
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        req.send()
            .await
            .map_err(|_| DOWN)?
            .error_for_status()
            .map_err(|_| DOWN)?
            .json()
            .await
            .map_err(|_| DOWN)
    }
}

fn valid_price(product: &Value, cents: i64) -> bool {
    let Some(prices) = product["prices"].as_array() else {
        return false;
    };
    let mut active = prices.iter().filter(|p| p["is_archived"] == false);
    let Some(price) = active.next() else {
        return false;
    };
    cents > 0
        && active.next().is_none()
        && product["is_archived"] == false
        && product["recurring_interval"] == "month"
        && product["recurring_interval_count"] == 1
        && price["amount_type"] == "fixed"
        && price["price_currency"] == "usd"
        && price["price_amount"].as_i64() == Some(cents)
}

pub async fn checkout(State(app): State<App>, h: HeaderMap) -> Result<Json<Value>, Failure> {
    let polar = Polar::load()?;
    let mut db = app.pool.get().await.map_err(|_| DOWN)?;
    let tx = db.transaction().await.map_err(|_| DOWN)?;
    let owner = tx.query_opt("SELECT a.id,a.sg_subject::text,a.plan FROM sessions s JOIN accounts a ON a.id=s.account_id WHERE s.digest=$1 AND s.scope='owner' AND s.expires_at>now() FOR UPDATE OF a", &[&bearer(&h)?])
        .await.map_err(|_| DOWN)?.ok_or(StatusCode::UNAUTHORIZED)?;
    let id: i64 = owner.get(0);
    let subject: String = owner.try_get(1).map_err(|_| DOWN)?;
    let external = format!("psms:{subject}");
    let plan = tx
        .query_one(
            "SELECT monthly_price_cents,available FROM plans WHERE id='plus'",
            &[],
        )
        .await
        .map_err(|_| DOWN)?;
    let cents = i64::from(plan.get::<_, i32>(0));
    let billing = tx.query_opt("SELECT coalesce(valid_until>now(),false),CASE WHEN checkout_expires>now() AND checkout_price=$2 THEN checkout_url END FROM polar_plus WHERE account_id=$1", &[&id,&cents]).await.map_err(|_| DOWN)?;
    if billing.as_ref().is_some_and(|r| r.get::<_, bool>(0)) {
        let session = polar
            .request(
                Method::POST,
                "/customer-sessions/",
                Some(json!({"external_customer_id":external,"return_url":RETURN})),
            )
            .await?;
        let url = session["customer_portal_url"].as_str().ok_or(DOWN)?;
        return Ok(Json(json!({"url":url})));
    }
    if !plan.get::<_, bool>(1) || owner.get::<_, String>(2) != "free" {
        return Err(StatusCode::CONFLICT);
    }
    if let Some(url) = billing.and_then(|r| r.get::<_, Option<String>>(1)) {
        return Ok(Json(json!({"url":url})));
    }
    let product = polar
        .request(Method::GET, &format!("/products/{}", polar.product), None)
        .await?;
    if !valid_price(&product, cents) {
        return Err(DOWN);
    }
    let checkout = polar.request(Method::POST,"/checkouts/", Some(json!({
        "products":[polar.product],"external_customer_id":external,"success_url":RETURN,
        "return_url":"https://www.papamo.dev/sms#plans","allow_trial":false,"allow_discount_codes":false
    }))).await?;
    if checkout["amount"].as_i64() != Some(cents) || checkout["currency"] != "usd" {
        return Err(DOWN);
    }
    let url = checkout["url"].as_str().ok_or(DOWN)?;
    let expires = checkout["expires_at"].as_str().ok_or(DOWN)?;
    tx.execute("INSERT INTO polar_plus(account_id,checkout_url,checkout_expires,checkout_price) VALUES($1,$2,$3::text::timestamptz,$4) ON CONFLICT(account_id) DO UPDATE SET checkout_url=excluded.checkout_url,checkout_expires=excluded.checkout_expires,checkout_price=excluded.checkout_price",&[&id,&url,&expires,&cents]).await.map_err(|_|DOWN)?;
    tx.commit().await.map_err(|_| DOWN)?;
    Ok(Json(json!({"url":url})))
}

#[derive(Deserialize)]
struct Customer {
    external_id: Option<String>,
    active_subscriptions: Vec<Subscription>,
}
#[derive(Deserialize)]
struct Subscription {
    product_id: Uuid,
    status: String,
    current_period_end: String,
}

pub async fn webhook(
    State(app): State<App>,
    h: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, Failure> {
    let polar = Polar::load()?;
    polar
        .hook
        .verify(&body, &h)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let event: Value = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    if event["type"] != "customer.state_changed" {
        return Ok(StatusCode::NO_CONTENT);
    }
    let Some(external) = event["data"]["external_id"].as_str() else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let Some(subject) = external
        .strip_prefix("psms:")
        .and_then(|s| Uuid::parse_str(s).ok())
    else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let mut db = app.pool.get().await.map_err(|_| DOWN)?;
    let tx = db.transaction().await.map_err(|_| DOWN)?;
    let Some(owner) = tx
        .query_opt(
            "SELECT id FROM accounts WHERE sg_subject=$1 FOR UPDATE",
            &[&subject.to_string()],
        )
        .await
        .map_err(|_| DOWN)?
    else {
        return Ok(StatusCode::NO_CONTENT);
    };
    // Re-read current provider state under the account lock so duplicates and out-of-order
    // deliveries converge even when cancellation and renewal cross in flight.
    let state: Customer = serde_json::from_value(
        polar
            .request(
                Method::GET,
                &format!("/customers/external/psms:{subject}/state"),
                None,
            )
            .await?,
    )
    .map_err(|_| DOWN)?;
    if state.external_id.as_deref() != Some(external) {
        return Err(DOWN);
    }
    let ends: Vec<String> = state
        .active_subscriptions
        .into_iter()
        .filter(|s| s.product_id == polar.product && s.status == "active")
        .map(|s| s.current_period_end)
        .collect();
    let id: i64 = owner.get(0);
    tx.execute("INSERT INTO polar_plus(account_id,valid_until) SELECT $1,max(e::timestamptz) FROM unnest($2::text[]) e ON CONFLICT(account_id) DO UPDATE SET valid_until=excluded.valid_until,checkout_url=CASE WHEN excluded.valid_until IS NOT NULL OR polar_plus.valid_until IS NOT NULL THEN NULL ELSE polar_plus.checkout_url END",&[&id,&ends]).await.map_err(|_|DOWN)?;
    tx.commit().await.map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_requires_one_active_monthly_price_matching_the_plan() {
        let price = json!({"is_archived":false,"amount_type":"fixed","price_currency":"usd","price_amount":300});
        let mut product = json!({"is_archived":false,"recurring_interval":"month","recurring_interval_count":1,"prices":[price.clone()]});
        assert!(valid_price(&product, 300));
        assert!(!valid_price(&product, 301));
        product["recurring_interval"] = json!("year");
        assert!(!valid_price(&product, 300));
        product["recurring_interval"] = json!("month");
        product["prices"].as_array_mut().unwrap().push(price);
        assert!(!valid_price(&product, 300));
        product["prices"][0]["is_archived"] = json!(true);
        assert!(valid_price(&product, 300));
    }
}
