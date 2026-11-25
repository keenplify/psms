use crate::{Archive, encryption};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    io::Write,
    path::{Path as FilePath, PathBuf},
    sync::Arc,
    time::Duration,
};
use subtle::ConstantTimeEq;

type AnyError = Box<dyn Error>;
const BATCH: usize = 8;
const RETRY: u64 = 300;
const DEFAULT_INTERVAL: u64 = 6 * 3600;
#[derive(clap::Subcommand)]
pub enum Command {
    /// On the receiving computer, create a private pairing file for one source archive.
    Pair {
        #[arg(long)]
        source: String,
        #[arg(long)]
        public_url: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// On the sending computer, import the private pairing file.
    Connect { pairing_file: PathBuf },
    /// Show local pairing and queue information without printing credentials.
    Status,
}
#[derive(Serialize, Deserialize)]
struct Pairing {
    source: String,
    endpoint: String,
    token: String,
}
fn sidecar(db: &FilePath, suffix: &str) -> PathBuf {
    let mut path = db.as_os_str().to_owned();
    path.push(suffix);
    path.into()
}
fn read<T: serde::de::DeserializeOwned>(p: &FilePath) -> Result<T, AnyError> {
    Ok(serde_json::from_slice(&fs::read(p)?)?)
}
fn write_new(p: &FilePath, value: &impl Serialize) -> Result<(), AnyError> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(p)?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    Ok(())
}
fn valid_source(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s != "sg"
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
fn verified(p: &Pairing) -> Result<(), AnyError> {
    let u = reqwest::Url::parse(&p.endpoint)?;
    let loopback = matches!(u.host_str(), Some("127.0.0.1" | "[::1]"));
    if !(valid_source(&p.source)
        && p.token.len() == 64
        && p.token.bytes().all(|b| b.is_ascii_hexdigit())
        && (u.scheme() == "https" || u.scheme() == "http" && loopback)
        && u.username().is_empty()
        && u.password().is_none()
        && u.query().is_none()
        && u.fragment().is_none()
        && u.path() == format!("/internal/sync/{}", p.source))
    {
        return Err("invalid pairing file or URL; use an HTTPS origin".into());
    }
    Ok(())
}
fn peers(db: &FilePath) -> PathBuf {
    sidecar(db, ".sync-peers")
}
pub fn setup(db: &FilePath, cmd: Command) -> Result<(), AnyError> {
    match cmd {
        Command::Pair {
            source,
            public_url,
            output,
        } => {
            let p = Pairing {
                endpoint: format!(
                    "{}/internal/sync/{}",
                    public_url.trim_end_matches('/'),
                    source
                ),
                source,
                token: crate::desktop::secret(),
            };
            verified(&p)?;
            let folder = peers(db);
            fs::create_dir_all(&folder)?;
            let peer = folder.join(&p.source);
            if peer.try_exists()? {
                return Err("source already paired; use the existing private pairing file".into());
            }
            write_new(&output, &p)?;
            write_new(&peer, &p.token)?;
            println!(
                "Pairing file created. Transfer it privately to the sender. Restart the receiver to load the pairing."
            );
        }
        Command::Connect { pairing_file } => {
            let p: Pairing = if pairing_file == FilePath::new("-") {
                serde_json::from_reader(std::io::stdin().lock())?
            } else {
                read(&pairing_file)?
            };
            verified(&p)?;
            write_new(&sidecar(db, ".sync-send"), &p)?;
            println!("Source paired. A running pSMS service picks it up within 15 seconds.");
        }
        Command::Status => {
            println!("Receiving from {} source(s).", load_peers(db)?.len());
            if sidecar(db, ".sync-send").try_exists()? {
                let p: Pairing = read(&sidecar(db, ".sync-send"))?;
                println!("Sending as {}.", p.source);
                if db.try_exists()? {
                    let conn = rusqlite::Connection::open_with_flags(
                        db,
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                    )?;
                    if conn.table_exists(None, "sync_queue")? {
                        let count: i64 =
                            conn.query_row("SELECT count(*) FROM sync_queue", [], |r| r.get(0))?;
                        println!("Pending messages: {count}");
                    }
                }
            }
        }
    };
    Ok(())
}
fn load_peers(db: &FilePath) -> Result<BTreeMap<String, String>, AnyError> {
    let dir = peers(db);
    let mut tokens = BTreeMap::new();
    if dir.try_exists()? {
        for entry in fs::read_dir(dir)? {
            let e = entry?;
            let name = e.file_name().into_string().map_err(|_| "invalid source")?;
            if !valid_source(&name) {
                return Err("invalid source name".into());
            }
            let token: String = read(&e.path())?;
            if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("invalid source credential".into());
            }
            tokens.insert(name, format!("Bearer {token}"));
        }
    }
    Ok(tokens)
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Change {
    pub(crate) revision: i64,
    pub(crate) id: uuid::Uuid,
    pub(crate) snapshot: Option<Snapshot>,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub(crate) contact_id: Option<String>,
    pub(crate) sender: String,
    pub(crate) body: String,
    pub(crate) archived_at: i64,
    pub(crate) starred: bool,
}

fn spare_capacity() -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    if sys.available_memory() < 192 * 1024 * 1024 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let Ok(n) = std::thread::available_parallelism() else {
            return false;
        };
        sysinfo::System::load_average().one < 0.65 * n.get() as f64
    }
    #[cfg(not(target_os = "linux"))]
    {
        sys.refresh_cpu_usage();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        sys.refresh_cpu_usage();
        !sys.cpus().is_empty() && sys.global_cpu_usage() < 65.0
    }
}
async fn quiet() -> bool {
    tokio::task::spawn_blocking(spare_capacity)
        .await
        .unwrap_or(false)
}

pub fn configure(a: &Archive, db_path: &FilePath) -> Result<Router<Archive>, AnyError> {
    let tokens = Arc::new(load_peers(db_path)?);
    let mut routes = Router::new();
    if !tokens.is_empty() {
        a.db.lock()
            .map_err(|_| "database lock")?
            .execute_batch(include_str!("sync_hub.sql"))?;
        routes = routes
            .route(
                "/internal/sync/{source}",
                post(
                    move |State(a): State<Archive>,
                          Path(source): Path<String>,
                          headers: HeaderMap,
                          Json(changes): Json<Vec<Change>>| {
                        accept_batch(a, source, headers, changes, tokens.clone())
                    },
                ),
            )
            .layer(DefaultBodyLimit::max(2 * 1024 * 1024));
    }
    let send = sidecar(db_path, ".sync-send");
    if send.try_exists()? {
        start_source(a, &send)?;
    } else {
        let archive = a.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                if send.is_file() && start_source(&archive, &send).is_ok() {
                    break;
                }
            }
        });
    }
    Ok(routes)
}
fn start_source(a: &Archive, send: &FilePath) -> Result<(), AnyError> {
    let p: Pairing = read(send)?;
    verified(&p)?;
    let interval = match std::env::var(crate::env_name!("SYNC_INTERVAL_SECONDS")) {
        Ok(v) => v.parse()?,
        Err(std::env::VarError::NotPresent) => DEFAULT_INTERVAL,
        Err(e) => return Err(e.into()),
    };
    if !(1..=604800).contains(&interval) {
        return Err("sync interval must be 1 to 604800 seconds".into());
    }
    {
        let mut db = a.db.lock().map_err(|_| "database lock")?;
        let tx = db.transaction()?;
        tx.execute_batch(include_str!("sync_source.sql"))?;
        let prior: Option<String> = tx
            .query_row("SELECT region FROM sync_schedule WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        match prior {
            Some(s) if s != p.source => {
                return Err("archive already belongs to a different source".into());
            }
            None => {
                tx.execute(
                    "INSERT INTO sync_schedule VALUES(1,unixepoch(),?1)",
                    [&p.source],
                )?;
                tx.execute_batch("INSERT INTO sync_queue(message_id) SELECT id FROM messages WHERE sender IN ('papamo:encrypted:v1','papamo:openpgp:v1');INSERT OR IGNORE INTO sync_queue(message_id) SELECT id FROM deleted_messages;")?;
            }
            _ => {}
        }
        tx.commit()?;
    }
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    tokio::spawn(worker(a.clone(), p, http, interval));
    Ok(())
}
async fn worker(a: Archive, p: Pairing, http: reqwest::Client, interval: u64) {
    loop {
        let delay = a
            .query(move |db| {
                db.query_row(
                    "SELECT max(0,completed_at+coalesce((SELECT seconds FROM sync_interval WHERE id=1),?1)-unixepoch()) FROM sync_schedule WHERE id=1",
                    [interval as i64],
                    |r| r.get::<_, i64>(0).map(|n| n as u64),
                )
            })
            .await
            .unwrap_or(RETRY);
        if delay > 0 {
            tokio::time::sleep(Duration::from_secs(delay.min(30))).await;
            continue;
        }
        if !quiet().await {
            tokio::time::sleep(Duration::from_secs(RETRY)).await;
            continue;
        }
        match send_batch(&a, &p, &http).await {
            Ok(true) => tokio::time::sleep(Duration::from_secs(1)).await,
            Ok(false) => {
                if a.query(|db| {
                    db.execute(
                        "UPDATE sync_schedule SET completed_at=unixepoch() WHERE id=1",
                        [],
                    )
                })
                .await
                .is_err()
                {
                    tokio::time::sleep(Duration::from_secs(RETRY)).await;
                }
            }
            Err(()) => {
                eprintln!("Archive sync deferred; queued changes retained.");
                tokio::time::sleep(Duration::from_secs(RETRY)).await;
            }
        }
    }
}
async fn send_batch(a: &Archive, p: &Pairing, http: &reqwest::Client) -> Result<bool, ()> {
    let changes = if let Some(target) = &a.remote {
        let after = a
            .query(|db| {
                db.query_row(
                    "SELECT revision FROM external_sync_cursor WHERE id=1",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .map_err(|_| ())?;
        target.changes(after, BATCH).await.map_err(|_| ())?
    } else {
        a.query(read_batch).await.map_err(|_| ())?
    };
    let Some(last) = changes.last().map(|c| c.revision) else {
        return Ok(false);
    };
    let response = http
        .post(&p.endpoint)
        .bearer_auth(&p.token)
        .json(&changes)
        .send()
        .await
        .map_err(|_| ())?;
    if response.status() != StatusCode::OK || response.json::<i64>().await.map_err(|_| ())? != last
    {
        return Err(());
    }
    let remote = a.remote.is_some();
    a.query(move |db| {
        if remote {
            db.execute(
                "UPDATE external_sync_cursor SET revision=?1 WHERE id=1 AND revision<?1",
                [last],
            )
        } else {
            db.execute("DELETE FROM sync_queue WHERE seq<=?1", [last])
        }
    })
    .await
    .map_err(|_| ())?;
    Ok(true)
}

async fn accept_batch(
    a: Archive,
    source: String,
    headers: HeaderMap,
    changes: Vec<Change>,
    tokens: Arc<BTreeMap<String, String>>,
) -> Result<Json<i64>, StatusCode> {
    let expected = tokens.get(&source).ok_or(StatusCode::UNAUTHORIZED)?;
    let provided = headers
        .get("authorization")
        .map(|v| v.as_bytes())
        .unwrap_or_default();
    if !bool::from(expected.as_bytes().ct_eq(provided)) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let ack = changes.last().ok_or(StatusCode::BAD_REQUEST)?.revision;
    if changes.len() > BATCH
        || changes.iter().any(|change| {
            change.revision < 1
                || change.snapshot.as_ref().is_some_and(|msg| {
                    !encryption::accepts(&msg.sender, &msg.body)
                        || msg.contact_id.as_ref().is_some_and(|id| {
                            id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit())
                        })
                })
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if !quiet().await {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    if let Some(target) = &a.remote {
        target
            .apply_replicas(&source, changes)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        return Ok(Json(ack));
    }
    a.query(move |db| {
        let tx=db.unchecked_transaction()?;
        for change in changes {
            let msg=change.snapshot.as_ref();
            let applied=tx.execute("INSERT INTO replicated_messages(region,id,revision,sender,body,archived_at,starred)
                VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(region,id) DO UPDATE SET
                revision=excluded.revision,sender=excluded.sender,body=excluded.body,
                archived_at=excluded.archived_at,starred=excluded.starred
                WHERE excluded.revision>replicated_messages.revision",
                params![source,change.id.to_string(),change.revision,msg.map(|m|&m.sender),
                    msg.map(|m|&m.body),msg.map(|m|m.archived_at),msg.is_some_and(|m|m.starred)])?;
            if applied == 1 {
                let id = change.id.to_string();
                tx.execute("DELETE FROM replicated_message_contacts WHERE region=?1 AND message_id=?2", params![source,id])?;
                if let Some((contact,time))=msg.and_then(|m|m.contact_id.as_ref().map(|c|(c.to_ascii_lowercase(),m.archived_at))) {
                    tx.execute("INSERT INTO replicated_contacts(region,id,first_seen_at,last_seen_at) VALUES(?3,?1,?2,?2) ON CONFLICT(region,id) DO UPDATE SET first_seen_at=min(first_seen_at,excluded.first_seen_at),last_seen_at=max(last_seen_at,excluded.last_seen_at)",params![contact,time,source])?;
                    tx.execute("INSERT INTO replicated_message_contacts(region,message_id,contact_id) VALUES(?1,?2,?3)",params![source,id,contact])?;
                }
            }
        }
        tx.commit()
    }).await?;
    Ok(Json(ack))
}
fn read_batch(db: &rusqlite::Connection) -> rusqlite::Result<Vec<Change>> {
    let mut stmt = db.prepare(
        "SELECT q.seq,q.message_id,m.sender,m.body,m.archived_at,coalesce(f.starred,0),c.contact_id
        FROM sync_queue q LEFT JOIN messages m ON m.id=q.message_id
        LEFT JOIN message_contacts c ON c.message_id=q.message_id LEFT JOIN message_flags f ON f.message_id=q.message_id ORDER BY q.seq LIMIT ?1",
    )?;
    stmt.query_map([BATCH as i64], |row| {
        let id: String = row.get(1)?;
        let id = id.parse().map_err(|_| rusqlite::Error::InvalidQuery)?;
        let sender: Option<String> = row.get(2)?;
        let snapshot = match sender {
            Some(sender)
                if matches!(sender.as_str(), "papamo:encrypted:v1" | "papamo:openpgp:v1") =>
            {
                Some(Snapshot {
                    contact_id: row.get(6)?,
                    sender,
                    body: row.get(3)?,
                    archived_at: row.get(4)?,
                    starred: row.get(5)?,
                })
            }
            _ => None,
        };
        Ok(Change {
            revision: row.get(0)?,
            id,
            snapshot,
        })
    })?
    .collect()
}
