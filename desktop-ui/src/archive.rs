#[path = "../../src/decrypt.rs"]
mod actions;
pub use actions::decrypt;
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
pub fn read(
    db: &Connection,
    page: i64,
    starred: bool,
    key: Option<&str>,
) -> rusqlite::Result<(Vec<Thread>, bool)> {
    let mut stmt=db.prepare("SELECT m.id,m.sender,m.body,strftime('%Y-%m-%d %H:%M',m.archived_at,'unixepoch','localtime'),coalesce(f.starred,0)
        FROM messages m LEFT JOIN message_flags f ON m.id=f.message_id WHERE ?1=0 OR f.starred=1 ORDER BY m.rowid DESC LIMIT ?2 OFFSET ?3")?;
    let mut rows = stmt.query((starred, (LIMIT + 1) as i64, page * LIMIT as i64))?;
    let mut records = Vec::new();
    while let Some(row) = rows.next()? {
        records.push((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ));
    }
    Ok(group(records, key))
}
pub type Record = (String, String, String, String, bool);
pub fn group(records: Vec<Record>, key: Option<&str>) -> (Vec<Thread>, bool) {
    let older = records.len() > LIMIT;
    let mut threads: Vec<Thread> = Vec::new();
    let mut groups = HashMap::new();
    for (id, provider, body, time, starred) in records.into_iter().take(LIMIT) {
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
            time,
            starred,
            locked,
            pgp,
        });
    }
    (threads, older)
}
