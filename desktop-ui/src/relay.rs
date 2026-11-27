//! Reply from anywhere: this Mac sends the texts queued in an inbox, wherever that inbox lives.
//! It works with a papamo.dev account (a computer token, texts sealed with the archive key) and
//! with any self-hosted psms server reachable over a tunnel such as Cloudflare or ngrok (an
//! owner or API token). It runs in the background while pSMS is open, sends through Messages,
//! and saves each sent text back to that inbox so it shows up live in every conversation view.
//!
//! The server URL is kept in the settings file. The token and archive key stay in the login
//! Keychain, so the relay keeps working after a restart without the words being typed again.
use crate::replies;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

pub const HOSTED: &str = "https://na.sms.papamo.dev/free/v1";
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS desktop_relay(id INTEGER PRIMARY KEY CHECK(id=1), base TEXT NOT NULL, checked_at INTEGER, sent INTEGER NOT NULL DEFAULT 0, error TEXT)";
#[cfg(target_os = "macos")]
const SERVICE: &str = "dev.papamo.psms.relay";

pub struct Relay {
    pub base: String,
    token: String,
    key: String,
}
pub struct Status {
    pub base: String,
    pub checked_at: Option<i64>,
    pub sent: i64,
    pub error: Option<String>,
}

#[cfg(target_os = "macos")]
fn keychain_write(account: &str, secret: &str) -> Result<(), &'static str> {
    use std::io::Write;
    // Commands go over stdin so the secret never appears in the process list.
    let mut child = std::process::Command::new("/usr/bin/security")
        .arg("-i")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "Could not open the Keychain.")?;
    let hex: String = secret.bytes().map(|b| format!("{b:02x}")).collect();
    writeln!(
        child.stdin.take().ok_or("Could not open the Keychain.")?,
        "add-generic-password -U -s {SERVICE} -a {account} -l \"pSMS reply relay\" -X {hex}"
    )
    .map_err(|_| "Could not open the Keychain.")?;
    match child.wait() {
        Ok(s) if s.success() => Ok(()),
        _ => Err("Could not save to the Keychain."),
    }
}
#[cfg(target_os = "macos")]
fn keychain_read(account: &str) -> Option<String> {
    let out = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", SERVICE, "-a", account, "-w"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8(out.stdout).ok())
        .flatten()
        .map(|s| s.trim_end().to_string())
}
#[cfg(target_os = "macos")]
fn keychain_delete(account: &str) {
    let _ = std::process::Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-s", SERVICE, "-a", account])
        .output();
}
#[cfg(not(target_os = "macos"))]
fn keychain_write(_: &str, _: &str) -> Result<(), &'static str> {
    Err("Replying from anywhere needs pSMS for Mac.")
}
#[cfg(not(target_os = "macos"))]
fn keychain_read(_: &str) -> Option<String> {
    None
}
#[cfg(not(target_os = "macos"))]
fn keychain_delete(_: &str) {}

/// Keychain items are named by server, without the path or any credentials.
fn account(base: &str) -> String {
    url::Url::parse(base)
        .ok()
        .and_then(|u| u.host_str().map(|h| format!("{h}{}", u.path())))
        .unwrap_or_else(|| base.into())
}

fn http() -> Result<(tokio::runtime::Runtime, reqwest::Client), &'static str> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not start the relay.")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not start the relay.")?;
    Ok((rt, client))
}
impl Relay {
    fn call(
        &self,
        method: reqwest::Method,
        route: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let (rt, client) = http().map_err(String::from)?;
        rt.block_on(async {
            let mut req = client
                .request(method, format!("{}{route}", self.base))
                .bearer_auth(&self.token);
            if let Some(body) = body {
                req = req.json(&body);
            }
            let res = req
                .send()
                .await
                .map_err(|_| "Could not reach the inbox server.".to_string())?;
            match res.status().as_u16() {
                200..=299 => {}
                401 | 403 => {
                    return Err(
                        "The inbox no longer accepts this Mac. Set up replies again.".into(),
                    );
                }
                404 => {
                    return Err("This server has no outbox. Update it to the latest psms.".into());
                }
                code => return Err(format!("The inbox server returned HTTP {code}.")),
            }
            let text = res.text().await.unwrap_or_default();
            Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
        })
    }
    /// Checks the token, and that the key opens this inbox's messages when it has any.
    pub fn check(base: &str, token: &str, key: &str) -> Result<Relay, String> {
        let base = base.trim().trim_end_matches('/').to_string();
        let parsed =
            url::Url::parse(&base).map_err(|_| "Enter the server's API URL.".to_string())?;
        let local = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        if !(parsed.scheme() == "https" || (parsed.scheme() == "http" && local))
            || !parsed.username().is_empty()
            || parsed.query().is_some()
        {
            return Err(
                "Use an HTTPS address, such as your Cloudflare or ngrok URL ending in /v1.".into(),
            );
        }
        let relay = Relay {
            base,
            token: token.trim().to_string(),
            key: key.to_string(),
        };
        let page = relay.call(reqwest::Method::GET, "/messages?limit=20", None)?;
        let bodies: Vec<&str> = page["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["sender"] == "papamo:encrypted:v1")
            .filter_map(|m| m["body"].as_str())
            .collect();
        if !bodies.is_empty()
            && !bodies
                .iter()
                .any(|b| crate::archive::decrypt(key, b).is_some())
        {
            return Err("Those recovery words don't open this inbox.".into());
        }
        Ok(relay)
    }
    /// Sends what is waiting, reports each result, and saves sent texts back to the inbox.
    pub fn run_once(&self, codes: Option<&mut Option<i64>>) -> Result<usize, String> {
        let batch = self.call(reqwest::Method::POST, "/outbox/claim", None)?;
        if let (Some(seen), Some(newest)) = (codes, batch["newest"].as_i64()) {
            match *seen {
                Some(before) if newest > before => self.announce_codes(before),
                _ => {}
            }
            *seen = Some(newest);
        }
        let mut sent = 0;
        for item in batch["messages"].as_array().into_iter().flatten() {
            let Some(id) = item["id"].as_str() else {
                continue;
            };
            let text = match item["envelope"].as_str() {
                Some(envelope) => crate::cards::open(&self.key, envelope).and_then(|v| {
                    Some((
                        v["to"].as_str()?.to_string(),
                        v["body"].as_str()?.to_string(),
                    ))
                }),
                None => item["to"]
                    .as_str()
                    .zip(item["body"].as_str())
                    .map(|(t, b)| (t.to_string(), b.to_string())),
            };
            let outcome = match text {
                None => {
                    json!({"status": "failed", "error": "This Mac could not open the text. Check the recovery words."})
                }
                Some((to, body)) => match replies::send(&to, &body) {
                    Ok(()) => {
                        sent += 1;
                        if let Some((copy, request)) = replies::sent_copy(&self.key, &to, &body) {
                            let _ = self.call(reqwest::Method::POST, "/messages", Some(request));
                            let _ = self.call(
                                reqwest::Method::POST,
                                "/messages/read",
                                Some(json!({"ids": [copy]})),
                            );
                        }
                        json!({"status": "sent"})
                    }
                    Err(e) => json!({"status": "failed", "error": e}),
                },
            };
            let _ = self.call(
                reqwest::Method::POST,
                &format!("/outbox/{id}/result"),
                Some(outcome),
            );
        }
        Ok(sent)
    }
}

impl Relay {
    /// Announces one-time codes in texts newer than `after` (Unix seconds).
    fn announce_codes(&self, after: i64) {
        let Ok(page) = self.call(reqwest::Method::GET, "/messages?limit=5", None) else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default();
        for m in page["messages"].as_array().into_iter().flatten() {
            let at = m["received_at"]
                .as_i64()
                .or(m["archived_at"].as_i64())
                .unwrap_or_default();
            if at <= after || now - at > crate::otp::RECENT_SECS {
                continue;
            }
            if let (Some(id), Some(body)) = (m["id"].as_str(), m["body"].as_str())
                && let Some((sender, text, false)) = crate::archive::open_message(&self.key, body)
                && let Some(code) = crate::otp::code(&text)
            {
                crate::otp::announce(id, &sender, &code);
            }
        }
    }
}

/// A computer token for a papamo.dev account, from a short-lived sign-in.
pub fn computer_token(signed_in: &str) -> Result<String, String> {
    let (rt, client) = http().map_err(String::from)?;
    rt.block_on(async {
        let v: Value = client
            .post(format!("{HOSTED}/computers"))
            .bearer_auth(signed_in)
            .send()
            .await
            .map_err(|_| "Could not reach your account. Try again.".to_string())
            .and_then(|r| match r.status().as_u16() {
                402 => Err(
                    "Your plan does not include replying from anywhere. See papamo.dev/sms#plans."
                        .to_string(),
                ),
                s if (200..300).contains(&s) => Ok(r),
                _ => Err("Could not add this Mac to your account. Sign in again.".to_string()),
            })?
            .json()
            .await
            .map_err(|_| "Invalid account response.".to_string())?;
        v["token"]
            .as_str()
            .filter(|t| t.len() == 64)
            .map(String::from)
            .ok_or_else(|| "Invalid account response.".into())
    })
}

pub fn save(db: &Connection, relay: &Relay) -> Result<(), &'static str> {
    keychain_write(
        &account(&relay.base),
        &json!({"token": relay.token, "key": relay.key}).to_string(),
    )?;
    db.execute_batch(SCHEMA)
        .and_then(|_| {
            db.execute(
                "INSERT INTO desktop_relay(id,base) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET base=excluded.base, checked_at=NULL, sent=0, error=NULL",
                [&relay.base],
            )
        })
        .map(drop)
        .map_err(|_| "Could not save the relay settings.")
}
pub fn forget(db: &Connection) {
    if let Some(s) = status(db) {
        keychain_delete(&account(&s.base));
    }
    let _ = db.execute_batch(SCHEMA);
    let _ = db.execute("DELETE FROM desktop_relay", []);
}
pub fn status(db: &Connection) -> Option<Status> {
    db.execute_batch(SCHEMA).ok()?;
    db.query_row(
        "SELECT base,checked_at,sent,error FROM desktop_relay WHERE id=1",
        [],
        |r| {
            Ok(Status {
                base: r.get(0)?,
                checked_at: r.get(1)?,
                sent: r.get(2)?,
                error: r.get(3)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}
fn load(db: &Connection) -> Option<Relay> {
    let base = status(db)?.base;
    let secret: Value = serde_json::from_str(&keychain_read(&account(&base))?).ok()?;
    Some(Relay {
        base,
        token: secret["token"].as_str()?.into(),
        key: secret["key"].as_str()?.into(),
    })
}

/// Checks the outbox every few seconds for as long as the app runs.
pub fn start(settings: PathBuf) {
    if !replies::SUPPORTED {
        return;
    }
    std::thread::spawn(move || {
        let mut cached: Option<Relay> = None;
        let mut newest: Option<i64> = None;
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let Ok(db) = Connection::open_with_flags(&settings, OpenFlags::SQLITE_OPEN_READ_WRITE)
            else {
                continue;
            };
            let Some(base) = status(&db).map(|s| s.base) else {
                cached = None;
                continue;
            };
            if cached.as_ref().is_none_or(|r| r.base != base) {
                cached = load(&db);
            }
            let Some(relay) = &cached else { continue };
            let result = relay.run_once(crate::otp::enabled(&db).then_some(&mut newest));
            let _ = db.execute(
                "UPDATE desktop_relay SET checked_at=unixepoch(), sent=sent+?1, error=?2 WHERE id=1",
                rusqlite::params![
                    *result.as_ref().unwrap_or(&0) as i64,
                    result.as_ref().err()
                ],
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keychain_names_and_server_addresses() {
        assert_eq!(
            account("https://na.sms.papamo.dev/free/v1"),
            "na.sms.papamo.dev/free/v1"
        );
        assert!(
            Relay::check("http://example.com/v1", "t", "k")
                .err()
                .unwrap()
                .contains("HTTPS")
        );
        assert!(Relay::check("https://user:pw@example.com/v1", "t", "k").is_err());
    }
}
#[cfg(test)]
mod live {
    use super::*;
    /// Run against a test server: PSMS_RELAY_TEST="<api url> <computer token>".
    #[test]
    #[ignore]
    fn claims_and_reports_texts_it_cannot_open() {
        let config = std::env::var("PSMS_RELAY_TEST").unwrap();
        let (base, token) = config.split_once(' ').unwrap();
        let relay = Relay::check(base, token, &"0".repeat(32)).unwrap();
        assert_eq!(relay.run_once(None).unwrap(), 0);
    }
}

/// Opens pSMS when you log in, so texts queued while the Mac was off send as soon as it is back.
#[cfg(target_os = "macos")]
fn login_agent() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Library/LaunchAgents/dev.papamo.psms.plist"))
}
#[cfg(target_os = "macos")]
pub fn opens_at_login() -> bool {
    login_agent().is_some_and(|p| p.exists())
}
#[cfg(target_os = "macos")]
pub fn set_open_at_login(on: bool) -> Result<(), &'static str> {
    let agent = login_agent().ok_or("Could not find your home folder.")?;
    if !on {
        return match std::fs::remove_file(&agent) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err("Could not turn off opening at login.")
            }
            _ => Ok(()),
        };
    }
    let cli = std::env::current_exe()
        .ok()
        .map(|p| p.with_file_name("psms"))
        .filter(|p| p.exists())
        .ok_or("Could not find the psms program next to this app.")?;
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>dev.papamo.psms</string>
<key>ProgramArguments</key><array><string>{}</string><string>desktop</string></array>
<key>RunAtLoad</key><true/>
<key>ProcessType</key><string>Interactive</string>
</dict></plist>
"#,
        escape(&cli.to_string_lossy())
    );
    std::fs::create_dir_all(agent.parent().ok_or("Could not open LaunchAgents.")?)
        .and_then(|_| std::fs::write(&agent, plist))
        .map_err(|_| "Could not turn on opening at login.")
}
#[cfg(not(target_os = "macos"))]
pub fn opens_at_login() -> bool {
    false
}
#[cfg(not(target_os = "macos"))]
pub fn set_open_at_login(_: bool) -> Result<(), &'static str> {
    Err("Opening at login is set up by the installer on this system.")
}
