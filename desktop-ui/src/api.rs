//! The local archive's HTTP API, reached with the owner token like the CLI and MCP server.
//! Every change the desktop makes goes through it, so all surfaces share one path.
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

#[derive(Clone)]
pub struct Api {
    base: String,
    token: String,
}
impl Api {
    #[cfg(test)]
    pub fn new(base: &str, token: &str) -> Api {
        Api {
            base: base.into(),
            token: token.into(),
        }
    }
    /// Finds the running local service for the archive at `path`.
    pub fn local(path: &Path, settings: &Path) -> Option<Api> {
        let mut ready = path.as_os_str().to_os_string();
        ready.push(".ready");
        let addr: std::net::SocketAddr =
            std::fs::read_to_string(ready).ok()?.trim().parse().ok()?;
        let db = Connection::open_with_flags(settings, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
        let token: String = db
            .query_row(
                "SELECT owner FROM local_credentials WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .ok()?;
        let prefix = db
            .query_row("SELECT prefix FROM tunnel_runtime WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap_or_else(|_| "/v1".into());
        Some(Api {
            base: format!("http://{addr}{prefix}"),
            token,
        })
    }
    /// The API address, as browser extensions connect to it.
    pub fn base(&self) -> &str {
        &self.base
    }
    pub fn call(
        &self,
        method: reqwest::Method,
        route: &str,
        body: Option<Value>,
    ) -> Result<Value, ()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| ())?;
        runtime.block_on(async {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| ())?;
            let mut req = client
                .request(method, format!("{}{route}", self.base))
                .bearer_auth(&self.token);
            if let Some(body) = body {
                req = req.json(&body);
            }
            let res = req.send().await.map_err(|_| ())?;
            if !res.status().is_success() {
                return Err(());
            }
            let text = res.text().await.map_err(|_| ())?;
            Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
        })
    }
    pub fn star(&self, id: &str, starred: bool) -> Result<(), ()> {
        self.call(
            reqwest::Method::PATCH,
            &format!("/messages/{id}"),
            Some(json!({"starred": starred})),
        )
        .map(drop)
    }
    pub fn mark_read(&self, ids: &[String]) -> Result<(), ()> {
        if ids.is_empty() {
            return Ok(());
        }
        self.call(
            reqwest::Method::POST,
            "/messages/read",
            Some(json!({"ids": ids})),
        )
        .map(drop)
    }
    /// Saves a sealed manual card, or clears it with `None` so the phone's copy shows again.
    pub fn save_card(&self, contact: &str, envelope: Option<String>) -> Result<(), ()> {
        self.call(
            reqwest::Method::PUT,
            &format!("/contacts/{contact}/card"),
            Some(json!({"envelope": envelope})),
        )
        .map(drop)
    }
    /// Saves a card read from this Mac's Contacts as the device's copy.
    pub fn save_device_card(&self, contact: &str, envelope: String) -> Result<(), ()> {
        self.call(
            reqwest::Method::PUT,
            &format!("/contacts/{contact}/card"),
            Some(json!({"envelope": envelope, "copy": "device"})),
        )
        .map(drop)
    }
    /// Connects this archive to papamo.dev Plus with a computer token.
    pub fn connect_cloud(&self, token: &str) -> Result<(), ()> {
        self.call(
            reqwest::Method::PUT,
            "/cloud",
            Some(json!({"token": token})),
        )
        .map(drop)
    }
    pub fn disconnect_cloud(&self) -> Result<(), ()> {
        self.call(reqwest::Method::DELETE, "/cloud", None).map(drop)
    }
    pub fn restore_cloud(&self) -> Result<(), ()> {
        self.call(reqwest::Method::POST, "/cloud/restore", None)
            .map(drop)
    }
}
