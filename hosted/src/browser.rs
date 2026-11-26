use super::*;
use axum::http::header;
use axum::response::{
    IntoResponse,
    sse::{Event, Sse},
};
use tokio::sync::broadcast;

async fn account(app: &App, h: &HeaderMap) -> Result<i64, Failure> {
    app.pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .query_opt(
            "SELECT account_id FROM browser_access WHERE digest=$1 AND expires_at>now()",
            &[&bearer(h)?],
        )
        .await
        .map_err(|_| DOWN)?
        .map(|r| r.get(0))
        .ok_or(DENIED)
}
pub(super) async fn issue(State(a): State<App>, h: HeaderMap) -> Reply {
    let user = identity(&a, &h, false).await?;
    let (token, digest) = new_token();
    a.pool.get().await.map_err(|_|DOWN)?.execute("INSERT INTO browser_access VALUES ($1,$2,now()+interval '7 days') ON CONFLICT(account_id) DO UPDATE SET digest=excluded.digest,expires_at=excluded.expires_at", &[&user,&digest]).await.map_err(|_|DOWN)?;
    Ok(Json(json!({"token":token})))
}
pub(super) async fn revoke(State(a): State<App>, h: HeaderMap) -> Result<StatusCode, Failure> {
    let user = identity(&a, &h, false).await?;
    a.pool
        .get()
        .await
        .map_err(|_| DOWN)?
        .execute("DELETE FROM browser_access WHERE account_id=$1", &[&user])
        .await
        .map_err(|_| DOWN)?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn recent(
    State(a): State<App>,
    h: HeaderMap,
) -> Result<impl IntoResponse, Failure> {
    let user = account(&a, &h).await?;
    let rows=a.pool.get().await.map_err(|_|DOWN)?.query("SELECT body, floor(extract(epoch FROM received_at))::bigint FROM archived_messages WHERE account_id=$1 AND sender='papamo:encrypted:v1' AND received_at BETWEEN now()-interval '5 minutes' AND now() ORDER BY received_at DESC,id LIMIT 25",&[&user]).await.map_err(|_|DOWN)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(
            rows.iter()
                .map(|r| json!({"body":r.get::<_,String>(0),"archived_at":r.get::<_,i64>(1)}))
                .collect::<Vec<_>>(),
        ),
    ))
}
pub(super) async fn listen(conf: Config, tx: broadcast::Sender<Option<i64>>) {
    loop {
        if let Ok((client, mut conn)) = conf.connect(NoTls).await {
            let notifications = tx.clone();
            let worker = tokio::spawn(async move {
                while let Some(Ok(msg)) =
                    futures_util::future::poll_fn(|cx| conn.poll_message(cx)).await
                {
                    if let tokio_postgres::AsyncMessage::Notification(n) = msg
                        && n.channel() == "psms_arrival"
                        && let Ok(id) = n.payload().parse::<i64>()
                    {
                        let _ = notifications.send(Some(id));
                    }
                }
            });
            if client.batch_execute("LISTEN psms_arrival").await.is_ok() {
                let _ = tx.send(None);
                let _ = worker.await;
            } else {
                worker.abort();
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
pub(super) async fn events(
    State(a): State<App>,
    h: HeaderMap,
) -> Result<impl IntoResponse, Failure> {
    let id = account(&a, &h).await?;
    let rx = a.arrivals.subscribe();
    let stream =
        futures_util::stream::unfold((rx, a, h, true), move |(mut rx, a, h, first)| async move {
            let event = if first {
                Event::default().event("ready").data("1")
            } else {
                let wait = async {
                    loop {
                        match rx.recv().await {
                            Ok(Some(user)) if user != id => continue,
                            Ok(Some(_)) => return Some("arrival"),
                            Ok(None) | Err(broadcast::error::RecvError::Lagged(_)) => {
                                return Some("ready");
                            }
                            Err(broadcast::error::RecvError::Closed) => return None,
                        }
                    }
                };
                match tokio::time::timeout(Duration::from_secs(15), wait).await {
                    Ok(Some(name)) => Event::default().event(name).data("1"),
                    Ok(None) => return None,
                    Err(_) => Event::default().comment("keepalive"),
                }
            };
            if account(&a, &h).await.ok() != Some(id) {
                return None;
            }
            Some((Ok::<_, std::convert::Infallible>(event), (rx, a, h, false)))
        });
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Sse::new(stream),
    ))
}
