// Prefer the current spelling even when explicitly empty.
#[macro_export]
macro_rules! env_name {
    ($suffix:literal) => {{
        const CURRENT: &str = concat!("PSMS_", $suffix);
        const LEGACY: &str = concat!("PAPAMO_", $suffix);
        if std::env::var_os(CURRENT).is_none() && std::env::var_os(LEGACY).is_some() {
            LEGACY
        } else {
            CURRENT
        }
    }};
}
#[allow(dead_code)]
mod app_tokens;
mod cli;
mod contacts;
mod daemon;
use clap::Parser;
mod decrypt;
mod desktop;
mod encryption;
mod hook_store;
mod inbox;
mod mcp;
mod native;
mod onboard;
mod sync;
#[cfg(windows)]
mod tray;
mod tunnel;
mod webhooks;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use std::{
    env,
    error::Error,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use subtle::ConstantTimeEq;
use uuid::Uuid;

#[derive(Clone)]
struct Archive {
    db: Arc<Mutex<Connection>>,
    credential: Arc<str>,
    owner: Option<Arc<str>>,
    replica_reader: Option<Arc<str>>,
    providers: Arc<Vec<encryption::Provider>>,
}

#[derive(Deserialize)]
struct Message {
    id: Uuid,
    sender: String,
    body: String,
    contact_id: Option<String>,
}

impl Archive {
    fn store(&self, message: Message) -> Result<StatusCode, Box<dyn Error + Send + Sync>> {
        let mut db = self.db.lock().map_err(|_| "database lock poisoned")?;
        let id = message.id.to_string();
        let tx = db.transaction()?;
        let inserted = tx.execute(
            "INSERT INTO messages (id, sender, body) SELECT ?1, ?2, ?3 WHERE NOT EXISTS (SELECT 1 FROM deleted_messages WHERE id=?1) ON CONFLICT(id) DO NOTHING",
            params![id, message.sender, message.body],
        )?;
        if inserted == 1 {
            if let Some(contact) = &message.contact_id {
                tx.execute("INSERT INTO contacts(id) VALUES(?1) ON CONFLICT(id) DO UPDATE SET last_seen_at=max(last_seen_at,unixepoch())", [contact])?;
                tx.execute(
                    "INSERT INTO message_contacts(message_id,contact_id) VALUES(?1,?2)",
                    params![id, contact],
                )?;
            }
            tx.commit()?;
            return Ok(StatusCode::CREATED);
        }
        let same: Option<bool> = tx
            .query_row(
                "SELECT sender = ?2 AND body = ?3 AND (SELECT contact_id FROM message_contacts WHERE message_id=?1) IS ?4 FROM messages WHERE id = ?1",
                params![id, message.sender, message.body, message.contact_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match same {
            Some(true) => StatusCode::OK,
            Some(false) => StatusCode::CONFLICT,
            None => StatusCode::GONE,
        })
    }
}

async fn receive(
    State(archive): State<Archive>,
    headers: HeaderMap,
    Json(mut message): Json<Message>,
) -> StatusCode {
    let provided = headers
        .get("authorization")
        .map(|value| value.as_bytes())
        .unwrap_or_default();
    if !bool::from(archive.credential.as_bytes().ct_eq(provided)) && !archive.is_owner(&headers) {
        return StatusCode::UNAUTHORIZED;
    }
    if !encryption::accepts(&message.sender, &message.body) {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    if let Some(id) = &mut message.contact_id {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return StatusCode::UNPROCESSABLE_ENTITY;
        }
        id.make_ascii_lowercase();
    }
    match tokio::task::spawn_blocking(move || archive.store(message)).await {
        Ok(Ok(status)) => status,
        _ => {
            eprintln!("Message storage failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

fn database_path(configured: Option<PathBuf>) -> Result<PathBuf, Box<dyn Error>> {
    if let Some(path) = configured {
        return Ok(path);
    }
    let folder = dirs::data_local_dir()
        .ok_or("could not locate the application data directory; set PSMS_DB_PATH")?
        .join("papamo-sms");
    let path = folder.join("papamo.sqlite3");
    let legacy = Path::new("papamo.sqlite3");
    if !path.try_exists()? && legacy.try_exists()? {
        return Err("found a working-directory archive; set PSMS_DB_PATH to its absolute path to keep using it".into());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&folder)?;
    Ok(path)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut cli = cli::Cli::parse();
    if let Some(cli::Command::ContactId { key_file }) = &cli.command {
        use sha2::{Digest, Sha256};
        let key = std::fs::read_to_string(key_file)?;
        let key = key.trim_end_matches(['\r', '\n']);
        if key.len() != 32 || !key.is_ascii() {
            return Err("expected a 32-character shortcut recovery key".into());
        }
        let mut sender = String::new();
        std::io::stdin().read_line(&mut sender)?;
        let sender = sender.trim_end_matches(['\r', '\n']);
        if sender.is_empty() {
            return Err("enter the exact sender ID or phone number".into());
        }
        println!(
            "{:x}",
            Sha256::digest(format!("psms-contact-v1\n{sender}\n{key}"))
        );
        return Ok(());
    }
    let db_path = database_path(cli.options.database.take())?;
    if let Some(cli::Command::Mcp {
        server,
        token_file,
        key_file,
    }) = cli.command
    {
        return mcp::run(
            &db_path,
            &cli.options.api_prefix,
            server,
            token_file,
            key_file,
        )
        .await;
    }
    if let Some(cli::Command::Sync { command }) = cli.command {
        return sync::setup(&db_path, command);
    }
    let preferred = "127.0.0.1:47832".parse()?;
    if let Some(cli::Command::Tunnel { provider }) = cli.command {
        let bind = cli
            .options
            .bind
            .or_else(|| daemon::running_address(&db_path).ok().flatten())
            .unwrap_or(preferred);
        return tunnel::run(provider, bind);
    }
    #[cfg(windows)]
    if cli.command.is_none()
        && std::env::current_exe()?
            .with_file_name("psms-desktop.exe")
            .is_file()
    {
        cli.command = Some(cli::Command::Desktop);
    }
    let worker = match cli.command {
        Some(cli::Command::Onboard { endpoint, output }) => {
            let token = match env::var(crate::env_name!("TOKEN")) {
                Ok(v) => v,
                Err(_) => Connection::open_with_flags(
                    &db_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?
                .query_row(
                    "SELECT upload FROM local_credentials WHERE singleton=1",
                    [],
                    |r| r.get(0),
                )?,
            };
            return onboard::prepare(&endpoint, &output, &token);
        }
        None | Some(cli::Command::Init) => return daemon::init(&db_path, &cli.options),
        Some(cli::Command::Status) => return daemon::status(&db_path),
        Some(cli::Command::Stop) => return daemon::stop(&db_path),
        Some(cli::Command::Token { role }) => {
            let conn =
                Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let sql = match role {
                cli::Role::Upload => "SELECT upload FROM local_credentials WHERE singleton=1",
                cli::Role::Owner => "SELECT owner FROM local_credentials WHERE singleton=1",
            };
            let secret: String = conn.query_row(sql, [], |r| r.get(0)).map_err(
                |_| "no generated credential available; use the configured environment credential",
            )?;
            println!("{secret}");
            return Ok(());
        }
        Some(cli::Command::Desktop) | Some(cli::Command::Settings) => {
            daemon::init(&db_path, &cli.options)?;
            native::launch(
                &db_path,
                matches!(cli.command, Some(cli::Command::Settings)),
            )?;
            return Ok(());
        }
        Some(cli::Command::DaemonWorker) => Some(daemon::Worker::acquire(&db_path)?),
        _ => None,
    };

    let configured_token = env::var_os(crate::env_name!("TOKEN"))
        .map(|v| v.into_string().map_err(|_| "invalid upload token"))
        .transpose()?;
    let prefix = cli.options.api_prefix;
    if !prefix.strip_prefix('/').is_some_and(|path| {
        path.split('/').all(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
    }) {
        return Err("PSMS_API_PREFIX requires an absolute path such as /v1".into());
    }
    let mut db = Connection::open(&db_path)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages (
        id TEXT PRIMARY KEY NOT NULL, sender TEXT NOT NULL, body TEXT NOT NULL,
        archived_at INTEGER NOT NULL DEFAULT (unixepoch())
    )",
    )?;
    db.execute_batch(
        "PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS contacts (
            id TEXT PRIMARY KEY NOT NULL, first_seen_at INTEGER NOT NULL DEFAULT (unixepoch()),
            last_seen_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE TABLE IF NOT EXISTS message_contacts (
            message_id TEXT PRIMARY KEY NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
            contact_id TEXT NOT NULL REFERENCES contacts(id)
        );
        CREATE INDEX IF NOT EXISTS message_contacts_by_contact ON message_contacts(contact_id);
        CREATE TABLE IF NOT EXISTS deleted_messages(id TEXT PRIMARY KEY NOT NULL);
        CREATE TRIGGER IF NOT EXISTS remember_deleted_message AFTER DELETE ON messages
        BEGIN INSERT OR IGNORE INTO deleted_messages(id) VALUES(OLD.id); END;
        CREATE TABLE IF NOT EXISTS message_flags (
            message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
            starred INTEGER NOT NULL
        )",
    )?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS sync_interval(id INTEGER PRIMARY KEY CHECK(id=1),seconds INTEGER NOT NULL CHECK(seconds BETWEEN 1 AND 604800))")?;
    db.execute_batch(app_tokens::SCHEMA)?;
    db.execute_batch(include_str!("webhooks.sql"))?;
    let configured_owner = env::var_os(crate::env_name!("OWNER_TOKEN"))
        .map(|s| s.into_string().map_err(|_| "invalid owner token"))
        .transpose()?;
    let (token, owner) = match configured_token {
        Some(token) => (token, configured_owner),
        None => {
            let (upload, generated_owner) = desktop::credentials(&mut db)?;
            (upload, Some(configured_owner.unwrap_or(generated_owner)))
        }
    };
    if token.len() < 32 || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("invalid PSMS_TOKEN".into());
    }
    if let Some(value) = &owner
        && (value == &token || value.len() < 32 || !value.bytes().all(|b| b.is_ascii_graphic()))
    {
        return Err("PSMS_OWNER_TOKEN must be separate from PSMS_TOKEN and contain at least 32 visible ASCII characters".into());
    }
    let replica_reader = env::var(crate::env_name!("REPLICA_READ_TOKEN")).ok();
    if let Some(reader) = &replica_reader
        && (reader.len() < 32
            || !reader.bytes().all(|b| b.is_ascii_graphic())
            || reader == &token
            || Some(reader) == owner.as_ref())
    {
        return Err("configure a separate replica operator credential of at least 32 visible ASCII characters".into());
    }
    let archive = Archive {
        replica_reader: replica_reader.map(|s| format!("Bearer {s}").into()),
        db: Arc::new(Mutex::new(db)),
        credential: format!("Bearer {token}").into(),
        owner: owner.map(|value| format!("Bearer {value}").into()),
        providers: Arc::new(encryption::providers()?),
    };
    webhooks::start(archive.clone())?;
    let sync_routes = sync::configure(&archive, &db_path)?;
    let app = Router::new()
        .route(
            "/.well-known/psms",
            get(|| async { Json(serde_json::json!({"service":"psms","version":1})) }),
        )
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .merge(inbox::routes(&prefix, archive.clone()))
        .route(&format!("{prefix}/messages"), post(receive))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .merge(sync_routes)
        .with_state(archive);
    let listener = match tokio::net::TcpListener::bind(cli.options.bind.unwrap_or(preferred)).await
    {
        Err(e) if cli.options.bind.is_none() && e.kind() == std::io::ErrorKind::AddrInUse => {
            tokio::net::TcpListener::bind("127.0.0.1:0").await?
        }
        result => result?,
    };
    #[cfg(windows)]
    let _tray = if let Some(w) = worker.as_ref() {
        Some(tray::start(&db_path, w.request_path())?)
    } else {
        None
    };
    eprintln!(
        "Listening on {} ({prefix}/messages)",
        listener.local_addr()?
    );
    if let Some(ref running) = worker {
        running.ready(listener.local_addr()?)?;
    }
    let stop_request = worker.as_ref().map(daemon::Worker::request_path);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            if let Some(path) = stop_request {
                daemon::await_stop(path).await;
            } else {
                std::future::pending::<()>().await;
            }
        })
        .await?;
    Ok(())
}
