use crate::{app_tokens, cli::ServerOptions, daemon, decrypt, hook_store, storage};
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
    /// Page through the inbox, decrypt locally, or star an archived message.
    Inbox {
        #[command(subcommand)]
        command: Inbox,
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
}
async fn api(
    path: &Path,
    options: &ServerOptions,
    route: &str,
    edit: Option<bool>,
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
    let req = match edit {
        Some(flag) => http.patch(url).json(&json!({"starred":flag})),
        None => http.get(url),
    };
    let res = req.bearer_auth(token).send().await?.error_for_status()?;
    if edit.is_some() {
        Ok(Value::Null)
    } else {
        Ok(res.json().await?)
    }
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
                    .map(std::fs::read_to_string)
                    .transpose()?
                    .as_deref()
                    .map(crate::recovery::key_from_backup)
                    .transpose()?;
                unlock(&mut entry, k.as_deref().map(str::trim_end));
                println!("{entry}");
            }
            Inbox::List {
                cursor,
                starred,
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
                    .map(std::fs::read_to_string)
                    .transpose()?
                    .as_deref()
                    .map(crate::recovery::key_from_backup)
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
                        && needle.as_ref().is_none_or(|n| {
                            ["sender", "body"].iter().any(|f| {
                                r[*f].as_str().is_some_and(|v| v.to_lowercase().contains(n))
                            })
                        })
                });
                println!("{page}");
            }
            other => {
                let (id, flag) = match other {
                    Inbox::Star { id } => (id, true),
                    Inbox::Unstar { id } => (id, false),
                    _ => unreachable!(),
                };
                api(path, opts, &format!("/messages/{id}"), Some(flag)).await?;
            }
        },
    }
    Ok(())
}
