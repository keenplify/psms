mod migrate;
pub use migrate::{local_state, lock, runtime};
mod archive;
mod read;
use clap::{Subcommand, ValueEnum};
use rbdc::db::{Connection, Driver};
use rbs::Value;
use serde::{Deserialize, Serialize};
use std::{error::Error, path::Path, time::Duration};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
#[derive(Debug, Copy, Clone, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Postgres,
    Mysql,
    Mariadb,
    Sqlserver,
}
#[derive(Clone)]
pub struct Target {
    engine: Engine,
    url: String,
}
impl Target {
    pub fn load(engine: Engine, file: &Path) -> Result<Self> {
        let url = std::fs::read_to_string(file)?.trim().to_string();
        if url.is_empty() {
            return Err("empty connection file".into());
        }
        Ok(Self { engine, url })
    }
    pub async fn open(&self) -> Result<Box<dyn Connection>> {
        let connect = async {
            let result = match self.engine {
                Engine::Postgres => rbdc_pg::PgDriver {}.connect(&self.url).await,
                Engine::Mysql | Engine::Mariadb => {
                    rbdc_mysql::MysqlDriver {}.connect(&self.url).await
                }
                Engine::Sqlserver => {
                    // Parse native ADO settings directly so the driver's convenience parser
                    // cannot switch certificate verification off.
                    let cfg = rbdc_mssql::tiberius::Config::from_ado_string(&self.url)
                        .map_err(|_| "invalid SQL Server connection settings")?;
                    rbdc_mssql::MssqlConnection::establish(&cfg)
                        .await
                        .map(|c| Box::new(c) as Box<dyn Connection>)
                }
            };
            result.map_err(|_| "connection failed; check host, database, TLS and credentials")
        };
        Ok(tokio::time::timeout(Duration::from_secs(12), connect)
            .await
            .map_err(|_| "database connection timed out")??)
    }
    async fn create(&self, c: &mut dyn Connection, name: &str, cols: &str) -> Result<()> {
        let sql = if self.engine == Engine::Sqlserver {
            format!("IF OBJECT_ID(N'{name}',N'U') IS NULL CREATE TABLE {name} ({cols})")
        } else {
            format!("CREATE TABLE IF NOT EXISTS {name} ({cols})")
        };
        c.exec(&sql, vec![])
            .await
            .map_err(|_| "schema setup failed; check CREATE permission")?;
        Ok(())
    }
    pub async fn verify(&self) -> Result<()> {
        let mut c = self.open().await?;
        c.ping().await.map_err(|_| "database ping failed")?;
        self.create(
            c.as_mut(),
            "psms_storage_probe",
            "id VARCHAR(36) NOT NULL PRIMARY KEY",
        )
        .await?;
        let id = uuid::Uuid::new_v4().to_string();
        c.begin().await?;
        let result = async {
            c.exec(
                "INSERT INTO psms_storage_probe(id) VALUES(?)",
                vec![Value::String(id.clone())],
            )
            .await?;
            let found: Vec<serde_json::Value> = rbs::from_value(
                c.exec_decode(
                    "SELECT id FROM psms_storage_probe WHERE id=?",
                    vec![Value::String(id.clone())],
                )
                .await?,
            )?;
            if found.len() != 1 || found[0]["id"].as_str() != Some(&id) {
                return Err("database read-back mismatch".into());
            }
            c.exec(
                "DELETE FROM psms_storage_probe WHERE id=?",
                vec![Value::String(id)],
            )
            .await?;
            Result::<()>::Ok(())
        }
        .await;
        c.rollback().await?;
        result?;
        self.open()
            .await?
            .ping()
            .await
            .map_err(|_| "reconnect verification failed")?;
        Ok(())
    }
}
#[derive(Subcommand)]
pub enum Command {
    #[command(hide = true)]
    DesktopPage {
        page: u32,
        #[arg(long)]
        starred: bool,
    },
    #[command(hide = true)]
    DesktopStar {
        id: uuid::Uuid,
        #[arg(action=clap::ArgAction::Set)]
        value: bool,
    },
    /// Verify that the configured destination and running service are ready.
    Status,
    /// Copy and verify a stopped SQLite archive, then select the new authoritative database.
    Switch {
        #[arg(value_enum)]
        engine: Engine,
        #[arg(long)]
        connection_file: std::path::PathBuf,
    },
    /// Connect and verify schema creation, transactional writes, read-back and reconnect.
    Check {
        #[arg(value_enum)]
        engine: Engine,
        #[arg(long)]
        connection_file: std::path::PathBuf,
    },
}
pub async fn run(path: &Path, command: Command) -> Result<()> {
    match command {
        Command::DesktopPage { page, starred } => {
            let target = runtime(path)
                .await?
                .ok_or("no external archive configured")?;
            println!("{}", target.desktop_page(page, starred).await?);
        }
        Command::DesktopStar { id, value } => {
            let target = runtime(path)
                .await?
                .ok_or("no external archive configured")?;
            if !target.edit(&id.to_string(), Some(value)).await? {
                return Err("message is unavailable".into());
            }
        }
        Command::Status => {
            let target = runtime(path).await?;
            let addr = crate::daemon::running_address(path)
                .map_err(|_| "cannot inspect running service")?
                .ok_or("start pSMS first")?;
            let response = reqwest::Client::builder()
                .timeout(Duration::from_secs(12))
                .build()?
                .get(format!("http://{addr}/readyz"))
                .send()
                .await?;
            if !response.status().is_success() {
                return Err("service readiness check failed".into());
            }
            if let Some(target) = target {
                target.verify().await?;
                println!("{:?} archive is active and writable.", target.engine);
            } else {
                println!("SQLite archive is active.");
            }
        }
        Command::Switch {
            engine,
            connection_file,
        } => return migrate::switch(path, engine, &connection_file).await,
        Command::Check {
            engine,
            connection_file,
        } => {
            Target::load(engine, &connection_file)?.verify().await?;
            println!("Database passed the connection and transactional read/write checks.");
        }
    }
    Ok(())
}
