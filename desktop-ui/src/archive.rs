use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rusqlite::Connection;
use std::collections::HashMap;

pub const LIMIT: usize = 40;
#[derive(Clone)]
pub struct Mail {
    pub id: String,
    pub text: String,
    pub time: String,
    pub starred: bool,
    pub locked: bool,
    pub pgp: bool,
}
pub struct Thread {
    pub key: String,
    pub title: String,
    pub items: Vec<Mail>,
}
pub fn decrypt(key: &str, body: &str) -> Option<(String, String)> {
    let bytes = STANDARD.decode(body).ok()?;
    if bytes.len() <= 28 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).ok()?;
    let clear = cipher
        .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&clear).ok()?;
    Some((
        json.get("sender")?.as_str()?.into(),
        json.get("body")?.as_str()?.into(),
    ))
}
pub fn read(
    db: &Connection,
    page: i64,
    starred: bool,
    key: Option<&str>,
) -> rusqlite::Result<(Vec<Thread>, bool)> {
    let mut stmt=db.prepare("SELECT m.id,m.sender,m.body,strftime('%Y-%m-%d %H:%M',m.archived_at,'unixepoch','localtime'),coalesce(f.starred,0)
        FROM messages m LEFT JOIN message_flags f ON m.id=f.message_id WHERE ?1=0 OR f.starred=1 ORDER BY m.rowid DESC LIMIT ?2 OFFSET ?3")?;
    let mut rows = stmt.query((starred, (LIMIT + 1) as i64, page * LIMIT as i64))?;
    let mut threads: Vec<Thread> = Vec::new();
    let mut groups = HashMap::new();
    let mut count = 0;
    while let Some(row) = rows.next()? {
        if count == LIMIT {
            return Ok((threads, true));
        }
        count += 1;
        let id: String = row.get(0)?;
        let provider: String = row.get(1)?;
        let body: String = row.get(2)?;
        let pgp = provider == "papamo:openpgp:v1";
        let content = if provider == "papamo:encrypted:v1" {
            key.and_then(|s| decrypt(s, &body))
        } else if pgp {
            None
        } else {
            Some((provider, body.clone()))
        };
        let locked = content.is_none();
        let (sender, text) = content.unwrap_or_else(|| ("Encrypted message".into(), body));
        let group = if locked {
            format!("locked/{id}")
        } else {
            format!("sender/{sender}")
        };
        let idx = *groups.entry(group.clone()).or_insert_with(|| {
            let i = threads.len();
            threads.push(Thread {
                key: group,
                title: sender.clone(),
                items: Vec::new(),
            });
            i
        });
        threads[idx].items.push(Mail {
            id,
            text,
            time: row.get(3)?,
            starred: row.get(4)?,
            locked,
            pgp,
        });
    }
    Ok((threads, false))
}
