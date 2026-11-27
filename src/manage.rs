use crate::{app_tokens, cards, cli::ServerOptions, daemon, decrypt, hook_store, storage};
use clap::{Subcommand, ValueEnum};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use serde_json::{Value, json};
use std::{
    error::Error,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;
type Result<T> = std::result::Result<T, Box<dyn Error>>;
#[derive(Subcommand)]
pub enum Action {
    /// Create, rotate, inspect or revoke scoped bearer tokens.
    Access {
        #[command(subcommand)]
        command: Access,
    },
    /// Add, list, rotate or remove event webhook destinations.
    Webhook {
        #[command(subcommand)]
        command: Hook,
    },
    /// Set the replication interval in seconds. Omit to restore the server default.
    SyncInterval {
        #[arg(value_parser=clap::value_parser!(u32).range(1..=604800))]
        seconds: Option<u32>,
    },
    /// Set the desktop appearance.
    Theme {
        #[arg(value_enum)]
        theme: Theme,
    },
    /// Page through the inbox, decrypt locally, star or mark messages read.
    Inbox {
        #[command(subcommand)]
        command: Inbox,
    },
    /// Show and edit contact names, phone numbers and emails, sealed with the archive key.
    Contact {
        #[command(subcommand)]
        command: Contact,
    },
}
#[derive(Subcommand)]
pub enum Contact {
    /// List contacts with their names. Needs the archive key to open cards.
    List {
        /// The 12 words, the original key, or the recovery kit PDF.
        #[arg(long)]
        key_file: PathBuf,
    },
    /// Show one contact by contact ID or exact sender.
    Show {
        /// A contact ID, or the sender's exact number or ID.
        contact: String,
        #[arg(long)]
        key_file: PathBuf,
    },
    /// Edit a contact. Given fields replace the current ones; phones and emails take label=value.
    Set {
        contact: String,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        org: Option<String>,
        #[arg(long)]
        title: Option<String>,
        /// Repeat for several numbers, for example --phone "mobile=+1 555 0100".
        #[arg(long = "phone")]
        phones: Vec<String>,
        #[arg(long = "email")]
        emails: Vec<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Drop your edits and show the phone's copy again.
    Reset {
        contact: String,
        #[arg(long)]
        key_file: PathBuf,
    },
}
#[derive(Clone, Copy, ValueEnum)]
pub enum Scope {
    Api,
    Browser,
}
impl Scope {
    fn id(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Browser => "browser",
        }
    }
}
#[derive(Clone, Copy, ValueEnum)]
pub enum Theme {
    System,
    Light,
    Dark,
}
#[derive(Subcommand)]
pub enum Access {
    Status {
        scope: Scope,
    },
    Create {
        scope: Scope,
        #[arg(long,value_parser=clap::value_parser!(u32).range(1..=36500))]
        days: Option<u32>,
    },
    Rotate {
        scope: Scope,
        #[arg(long,value_parser=clap::value_parser!(u32).range(1..=36500))]
        days: Option<u32>,
    },
    Revoke {
        scope: Scope,
    },
}
#[derive(Subcommand)]
pub enum Hook {
    List,
    Add { url: String },
    Rotate { id: Uuid },
    Remove { id: Uuid },
}
#[derive(Subcommand)]
pub enum Inbox {
    List {
        #[arg(long)]
        cursor: Option<i64>,
        #[arg(long,value_parser=clap::value_parser!(u32).range(1..=100),default_value="40")]
        limit: u32,
        #[arg(long)]
        starred: bool,
        /// Only messages not yet read on any inbox.
        #[arg(long)]
        unread: bool,
        #[arg(long)]
        search: Option<String>,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    Show {
        id: Uuid,
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    Star {
        id: Uuid,
    },
    Unstar {
        id: Uuid,
    },
    Read {
        id: Uuid,
    },
    Unread {
        id: Uuid,
    },
}
async fn api(
    path: &Path,
    options: &ServerOptions,
    route: &str,
    edit: Option<bool>,
) -> Result<Value> {
    match edit {
        Some(flag) => {
            call(
                path,
                options,
                reqwest::Method::PATCH,
                route,
                Some(json!({"starred": flag})),
            )
            .await
        }
        None => call(path, options, reqwest::Method::GET, route, None).await,
    }
}
async fn call(
    path: &Path,
    options: &ServerOptions,
    method: reqwest::Method,
    route: &str,
    body: Option<Value>,
) -> Result<Value> {
    let addr = daemon::running_address(path)?.ok_or("Start the daemon with psms init first.")?;
    let db =
        Connection::open_with_flags(storage::local_state(path), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let token = match std::env::var(crate::env_name!("OWNER_TOKEN")) {
        Ok(v) => v,
        Err(_) => db.query_row(
            "SELECT owner FROM local_credentials WHERE singleton=1",
            [],
            |r| r.get::<_, String>(0),
        )?,
    };
    let prefix = db
        .query_row("SELECT prefix FROM tunnel_runtime WHERE id=1", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap_or_else(|_| options.api_prefix.clone());
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let url = format!("http://{addr}{prefix}{route}");
    let mut req = http.request(method, url);
    if let Some(body) = body {
        req = req.json(&body);
    }
    let res = req.bearer_auth(token).send().await?.error_for_status()?;
    let text = res.text().await?;
    Ok(if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text)?
    })
}
/// Resolves a contact ID or an exact sender to the contact ID used by the archive.
fn contact_of(value: &str, key: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.len() == 64 && lower.bytes().all(|b| b.is_ascii_hexdigit()) {
        lower
    } else {
        crate::contact_id(value, key)
    }
}
fn label_value(raw: &str, fallback: &str) -> Value {
    match raw.split_once('=') {
        Some((label, value)) if !label.trim().is_empty() => {
            json!({"label": label.trim(), "value": value.trim()})
        }
        _ => json!({"label": fallback, "value": raw.trim()}),
    }
}
async fn contact_command(path: &Path, opts: &ServerOptions, command: Contact) -> Result<()> {
    let key_of = |file: &Path| crate::kit::key_from_file(file);
    let cards = |key: String| async move {
        let rows = call(path, opts, reqwest::Method::GET, "/contacts/cards", None).await?;
        let mut out = std::collections::HashMap::new();
        for row in rows["cards"].as_array().cloned().unwrap_or_default() {
            if let (Some(id), Some(card)) = (row["contact_id"].as_str(), cards::merged(&key, &row))
            {
                out.insert(id.to_string(), card);
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(out)
    };
    match command {
        Contact::List { key_file } => {
            let key = key_of(&key_file)?;
            let cards = cards(key).await?;
            let page = call(
                path,
                opts,
                reqwest::Method::GET,
                "/contacts?limit=100",
                None,
            )
            .await?;
            let mut out = Vec::new();
            for c in page["contacts"].as_array().cloned().unwrap_or_default() {
                let id = c["id"].as_str().unwrap_or_default().to_string();
                let card = cards.get(&id);
                out.push(json!({
                    "id": id,
                    "last_seen_at": c["last_seen_at"],
                    "card": card.map(|(v, _)| cards::summary(v)),
                    "source": card.map(|(_, s)| *s),
                }));
            }
            println!("{}", json!({"contacts": out}));
        }
        Contact::Show { contact, key_file } => {
            let key = key_of(&key_file)?;
            let id = contact_of(&contact, &key);
            let cards = cards(key).await?;
            let card = cards.get(&id);
            println!(
                "{}",
                json!({"id": id, "card": card.map(|(v, _)| cards::summary(v)), "source": card.map(|(_, s)| *s)})
            );
        }
        Contact::Set {
            contact,
            key_file,
            name,
            org,
            title,
            phones,
            emails,
            note,
        } => {
            let key = key_of(&key_file)?;
            let id = contact_of(&contact, &key);
            let mut card = cards(key.clone())
                .await?
                .remove(&id)
                .map(|(v, _)| v)
                .unwrap_or_else(|| json!({}));
            card["version"] = json!(1);
            for (field, value) in [("fn", name), ("org", org), ("title", title), ("note", note)] {
                if let Some(v) = value {
                    card[field] = json!(v);
                }
            }
            if !phones.is_empty() {
                card["tel"] =
                    Value::Array(phones.iter().map(|p| label_value(p, "mobile")).collect());
            } else {
                card["tel"] = Value::Array(cards::labeled(&card["tel"], "mobile"));
            }
            if !emails.is_empty() {
                card["email"] =
                    Value::Array(emails.iter().map(|e| label_value(e, "email")).collect());
            } else {
                card["email"] = Value::Array(cards::labeled(&card["email"], "email"));
            }
            let envelope = cards::seal(&key, &card).ok_or("could not seal the card")?;
            call(
                path,
                opts,
                reqwest::Method::PUT,
                &format!("/contacts/{id}/card"),
                Some(json!({"envelope": envelope})),
            )
            .await?;
            println!("{}", json!({"id": id, "card": cards::summary(&card)}));
        }
        Contact::Reset { contact, key_file } => {
            let key = key_of(&key_file)?;
            let id = contact_of(&contact, &key);
            call(
                path,
                opts,
                reqwest::Method::PUT,
                &format!("/contacts/{id}/card"),
                Some(json!({"envelope": null})),
            )
            .await?;
        }
    }
    Ok(())
}
pub async fn newest(path: &Path, opts: &ServerOptions) -> Result<Option<String>> {
    Ok(
        api(path, opts, "/messages?limit=1", None).await?["messages"][0]["id"]
            .as_str()
            .map(str::to_owned),
    )
}
fn unlock(entry: &mut Value, key: Option<&str>) {
    if entry["sender"] == "papamo:encrypted:v1"
        && let Some((sender, text)) = key.and_then(|k| decrypt::decrypt(k, entry["body"].as_str()?))
    {
        entry["sender"] = json!(sender);
        entry["body"] = json!(text);
    }
}
pub async fn run(path: &Path, opts: &ServerOptions, action: Action) -> Result<()> {
    let mut db = Connection::open_with_flags(
        storage::local_state(path),
        OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?;
    match action {
        Action::Access { command } => match command {
            Access::Status { scope } => println!(
                "{}",
                app_tokens::status(&db, scope.id())?.unwrap_or_else(|| "No token".into())
            ),
            Access::Revoke { scope } => app_tokens::revoke(&db, scope.id())?,
            command => {
                let create = matches!(command, Access::Create { .. });
                let (scope, days) = match command {
                    Access::Create { scope, days } | Access::Rotate { scope, days } => {
                        (scope.id(), days)
                    }
                    _ => unreachable!(),
                };
                let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
                if create == app_tokens::status(&tx, scope)?.is_some() {
                    return Err("Use create for a new token and rotate for an existing one.".into());
                }
                let token = app_tokens::issue(&tx, scope, days)?;
                tx.commit()?;
                println!("{token}");
            }
        },
        Action::Webhook { command } => match command {
            Hook::List => println!("{}", serde_json::to_string(&hook_store::list(&db)?)?),
            Hook::Add { url } => {
                if !hook_store::valid_url(&url) {
                    return Err("Enter an HTTPS webhook URL.".into());
                }
                let (id, token) = hook_store::create(&db, &url)?;
                println!("{}", json!({"id":id,"token":token}));
            }
            Hook::Rotate { id } => println!(
                "{}",
                hook_store::rotate(&db, &id.to_string())?.ok_or("Unknown webhook")?
            ),
            Hook::Remove { id } => {
                if !hook_store::remove(&db, &id.to_string())? {
                    return Err("Unknown webhook".into());
                }
            }
        },
        Action::SyncInterval { seconds } => {
            if let Some(s) = seconds {
                db.execute("INSERT INTO sync_interval VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET seconds=excluded.seconds",[s])?;
            } else {
                db.execute("DELETE FROM sync_interval WHERE id=1", [])?;
            }
        }
        Action::Theme { theme } => {
            db.execute_batch("CREATE TABLE IF NOT EXISTS desktop_appearance(id INTEGER PRIMARY KEY CHECK(id=1),dark INTEGER NOT NULL)")?;
            match theme {
                Theme::System => {
                    db.execute("DELETE FROM desktop_appearance WHERE id=1", [])?;
                }
                t => {
                    db.execute(
                        "INSERT OR REPLACE INTO desktop_appearance VALUES(1,?1)",
                        [matches!(t, Theme::Dark)],
                    )?;
                }
            }
        }
        Action::Inbox { command } => match command {
            Inbox::Show { id, key_file } => {
                let mut entry = api(path, opts, &format!("/messages/{id}"), None).await?;
                let k = key_file
                    .as_deref()
                    .map(crate::kit::key_from_file)
                    .transpose()?;
                unlock(&mut entry, k.as_deref().map(str::trim_end));
                println!("{entry}");
            }
            Inbox::List {
                cursor,
                starred,
                unread,
                limit,
                search,
                key_file,
            } => {
                let mut route = format!("/messages?limit={limit}");
                if let Some(n) = cursor {
                    route.push_str(&format!("&cursor={n}"));
                }
                let mut page = api(path, opts, &route, None).await?;
                let k = key_file
                    .as_deref()
                    .map(crate::kit::key_from_file)
                    .transpose()?;
                let needle = search.map(|n| n.to_lowercase());
                let rows = page["messages"]
                    .as_array_mut()
                    .ok_or("Invalid archive response")?;
                for row in rows.iter_mut() {
                    unlock(row, k.as_deref().map(str::trim_end));
                }
                rows.retain(|r| {
                    (!starred || r["starred"] == true)
                        && (!unread || r["read"] != true)
                        && needle.as_ref().is_none_or(|n| {
                            ["sender", "body"].iter().any(|f| {
                                r[*f].as_str().is_some_and(|v| v.to_lowercase().contains(n))
                            })
                        })
                });
                println!("{page}");
            }
            other => {
                let (id, body) = match other {
                    Inbox::Star { id } => (id, json!({"starred": true})),
                    Inbox::Unstar { id } => (id, json!({"starred": false})),
                    Inbox::Read { id } => (id, json!({"read": true})),
                    Inbox::Unread { id } => (id, json!({"read": false})),
                    _ => unreachable!(),
                };
                call(
                    path,
                    opts,
                    reqwest::Method::PATCH,
                    &format!("/messages/{id}"),
                    Some(body),
                )
                .await?;
            }
        },
        Action::Contact { command } => contact_command(path, opts, command).await?,
    }
    Ok(())
}
