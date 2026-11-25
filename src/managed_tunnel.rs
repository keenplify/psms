use crate::{Archive, public_host};
use ngrok::{config::ForwarderBuilder, prelude::EndpointInfo};
use rusqlite::{Connection, OptionalExtension, params};
use std::{net::SocketAddr, path::Path, time::Duration};
use tokio::task::JoinHandle;
use uuid::Uuid;

pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS desktop_tunnel(id INTEGER PRIMARY KEY CHECK(id=1),rev TEXT NOT NULL,provider TEXT NOT NULL,host TEXT NOT NULL,credential TEXT NOT NULL,state TEXT NOT NULL,result TEXT NOT NULL); CREATE TABLE IF NOT EXISTS tunnel_runtime(id INTEGER PRIMARY KEY CHECK(id=1),secret TEXT NOT NULL,prefix TEXT NOT NULL)";
#[derive(Clone)]
struct Config {
    rev: String,
    provider: String,
    host: String,
    credential: String,
}
type Failure = Box<dyn std::error::Error>;
pub async fn discover(path: &Path) -> Result<Option<String>, Failure> {
    let db = Connection::open_with_flags(
        crate::storage::local_state(path),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let secret: String = db.query_row("SELECT secret FROM tunnel_runtime WHERE id=1", [], |r| {
        r.get(0)
    })?;
    Ok(
        tokio::time::timeout(Duration::from_secs(14), public_host::detect(&secret))
            .await
            .ok()
            .flatten(),
    )
}
pub async fn configure(
    path: &Path,
    provider: String,
    host: String,
    mut credential: String,
) -> Result<String, Failure> {
    if !matches!(
        provider.as_str(),
        "ngrok" | "ngrok-default" | "cloudflare" | "direct"
    ) || !(public_host::valid_host(&host) || provider == "ngrok" && host.is_empty())
    {
        return Err("Enter a valid public hostname.".into());
    }
    if crate::daemon::running_address(path)?.is_none() {
        return Err("Start the local pSMS daemon first.".into());
    }
    let db = Connection::open_with_flags(
        crate::storage::local_state(path),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?;
    if provider == "direct" {
        credential.clear();
    } else if credential.trim().is_empty() {
        credential=db.query_row("SELECT credential FROM desktop_tunnel WHERE id=1 AND (provider=?1 OR (provider LIKE 'ngrok%' AND ?1 LIKE 'ngrok%'))",[&provider],|r|r.get(0)).optional()?.unwrap_or_default();
    }
    if provider.starts_with("ngrok") && credential.trim().is_empty() {
        return Err("Enter your ngrok authtoken first.".into());
    }
    let rev = Uuid::new_v4().to_string();
    db.execute("INSERT INTO desktop_tunnel VALUES(1,?1,?2,?3,?4,'pending','') ON CONFLICT(id) DO UPDATE SET rev=excluded.rev,provider=excluded.provider,host=excluded.host,credential=excluded.credential,state='pending',result=''",params![rev,provider,host,credential.trim()])?;
    for _ in 0..110 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let (state, value): (String, String) = db.query_row(
            "SELECT state,result FROM desktop_tunnel WHERE rev=?1",
            [&rev],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        match state.as_str() {
            "ready" => {
                return Ok(value);
            }
            "error" => return Err(value.into()),
            _ => {}
        }
    }
    Err("Connection timed out. Check the provider and try again.".into())
}
async fn report(a: &Archive, rev: &str, state: &'static str, value: &str) {
    let (r, v) = (rev.to_owned(), value.to_owned());
    let _ = a
        .query(move |db| {
            db.execute(
                "UPDATE desktop_tunnel SET state=?2,result=?3 WHERE rev=?1",
                params![r, state, v],
            )
        })
        .await;
}
pub fn watch(
    a: Archive,
    bind: SocketAddr,
    prefix: String,
    secret: String,
    path: std::path::PathBuf,
) {
    tokio::spawn(async move {
        let mut current = String::new();
        let mut active: Option<JoinHandle<()>> = None;
        loop {
            let config = a
                .query(|db| {
                    db.query_row(
                        "SELECT rev,provider,host,credential FROM desktop_tunnel WHERE id=1",
                        [],
                        |r| {
                            Ok(Config {
                                rev: r.get(0)?,
                                provider: r.get(1)?,
                                host: r.get(2)?,
                                credential: r.get(3)?,
                            })
                        },
                    )
                    .optional()
                })
                .await;
            if matches!(config, Ok(None)) && !current.is_empty() {
                if let Some(old) = active.take() {
                    old.abort();
                    let _ = old.await;
                }
                current.clear();
            }
            if let Ok(Some(c)) = config
                && current != c.rev
            {
                if let Some(old) = active.take() {
                    old.abort();
                    let _ = old.await;
                }
                current = c.rev.clone();
                let path = path.clone();
                let (archive, secret, prefix) = (a.clone(), secret.clone(), prefix.clone());
                active = Some(tokio::spawn(async move {
                    report(&archive, &c.rev, "pending", "").await;
                    if let Err(reason) = connect(&archive, &c, bind, &prefix, &secret, &path).await
                    {
                        report(&archive, &c.rev, "error", reason).await;
                    }
                }));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}
struct AbortForward(tokio::task::AbortHandle);
impl Drop for AbortForward {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn connect(
    a: &Archive,
    c: &Config,
    bind: SocketAddr,
    prefix: &str,
    secret: &str,
    path: &Path,
) -> Result<(), &'static str> {
    let mut host = c.host.clone();
    let mut forward = None;
    let mut child = None;
    let mut _guard = None;
    match c.provider.as_str() {
        "ngrok" | "ngrok-default" => {
            if c.credential.is_empty() {
                return Err("Enter your ngrok authtoken from the ngrok dashboard.");
            }
            let session = tokio::time::timeout(
                Duration::from_secs(15),
                ngrok::Session::builder().authtoken(&c.credential).connect(),
            )
            .await
            .map_err(|_| "ngrok connection timed out.")?
            .map_err(|_| "Could not sign in to ngrok. Check your authtoken.")?;
            let mut ep = session.http_endpoint();
            if !host.is_empty() {
                ep.domain(&host);
            }
            let requested = tokio::time::timeout(
                Duration::from_secs(15),
                ep.listen_and_forward(
                    format!("http://{bind}")
                        .parse()
                        .map_err(|_| "Invalid local address.")?,
                ),
            )
            .await
            .map_err(|_| "ngrok endpoint timed out.")?;
            let mut f = match requested {
                Ok(f) => f,
                Err(_) if c.provider == "ngrok-default" => tokio::time::timeout(
                    Duration::from_secs(12),
                    session.http_endpoint().listen_and_forward(
                        format!("http://{bind}")
                            .parse()
                            .map_err(|_| "Invalid local address.")?,
                    ),
                )
                .await
                .map_err(|_| "Assigned hostname timed out.")?
                .map_err(|_| "Could not open the ngrok assigned hostname.")?,
                Err(_) => {
                    return Err(
                        "ngrok could not open this name. Check your plan and domain reservation.",
                    );
                }
            };
            host = url::Url::parse(f.url())
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .ok_or("ngrok returned an invalid address.")?;
            _guard = Some(AbortForward(f.join().abort_handle()));
            forward = Some(f);
        }
        "cloudflare" => {
            child = Some(crate::cloudflare::start(path, &host, bind, &c.credential).await?);
        }
        "direct" => {}
        _ => return Err("Unknown provider."),
    }
    let http = public_host::client().map_err(|_| "Could not initialize HTTPS.")?;
    for _ in 0..8 {
        if let Some(ch) = &mut child
            && ch.try_wait().ok().flatten().is_some()
        {
            return Err("Cloudflare exited. Check the tunnel token and published route.");
        }
        if public_host::verify(&http, &host, secret).await {
            if c.provider == "ngrok-default" {
                let (rev, actual) = (c.rev.clone(), host.clone());
                a.query(move |db| {
                    db.execute(
                        "UPDATE desktop_tunnel SET provider='ngrok',host=?1 WHERE rev=?2",
                        params![actual, rev],
                    )
                })
                .await
                .map_err(|_| "Could not save the assigned hostname.")?;
            }
            report(
                a,
                &c.rev,
                "ready",
                &format!("https://{host}{prefix}/messages"),
            )
            .await;
            if let Some(mut f) = forward {
                let _ = f.join().await;
                return Err("ngrok disconnected. Reconnect to resume delivery.");
            }
            if let Some(mut ch) = child {
                let _ = ch.wait().await;
                return Err("Cloudflare disconnected. Reconnect to resume delivery.");
            }
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(
        "This hostname did not reach your archive over HTTPS. Check its DNS and tunnel or reverse-proxy route.",
    )
}
