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
    pub read: bool,
    /// Messages-style time for the conversation list: "9:54 AM", "Yesterday", a weekday or a date.
    pub when: String,
}
pub struct Thread {
    pub key: String,
    pub title: String,
    /// Hashed contact ID, when the sending shortcut provided one.
    pub contact: Option<String>,
    pub items: Vec<Mail>,
}
pub fn read(
    db: &Connection,
    page: i64,
    starred: bool,
    key: Option<&str>,
) -> rusqlite::Result<(Vec<Thread>, bool)> {
    let mut stmt=db.prepare("SELECT m.id,m.sender,m.body,strftime('%Y-%m-%d %H:%M',m.archived_at,'unixepoch','localtime'),coalesce(f.starred,0),(SELECT contact_id FROM message_contacts WHERE message_id=m.id),
        EXISTS(SELECT 1 FROM message_reads r WHERE r.message_id=m.id),
        CASE WHEN date(m.archived_at,'unixepoch','localtime')=date('now','localtime') THEN ltrim(strftime('%I:%M %p',m.archived_at,'unixepoch','localtime'),'0')
             WHEN date(m.archived_at,'unixepoch','localtime')=date('now','localtime','-1 day') THEN 'Yesterday'
             WHEN date(m.archived_at,'unixepoch','localtime')>date('now','localtime','-7 days') THEN
               CASE strftime('%w',m.archived_at,'unixepoch','localtime') WHEN '0' THEN 'Sunday' WHEN '1' THEN 'Monday' WHEN '2' THEN 'Tuesday' WHEN '3' THEN 'Wednesday' WHEN '4' THEN 'Thursday' WHEN '5' THEN 'Friday' ELSE 'Saturday' END
             ELSE strftime('%m/%d/%y',m.archived_at,'unixepoch','localtime') END
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
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
        ));
    }
    Ok(group(records, key))
}
/// id, provider, body, time, starred, contact ID, read, list time.
pub type Record = (
    String,
    String,
    String,
    String,
    bool,
    Option<String>,
    bool,
    String,
);
pub fn group(records: Vec<Record>, key: Option<&str>) -> (Vec<Thread>, bool) {
    let older = records.len() > LIMIT;
    let mut threads: Vec<Thread> = Vec::new();
    let mut groups = HashMap::new();
    for (id, provider, body, time, starred, contact, read, when) in records.into_iter().take(LIMIT)
    {
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
                contact: None,
                items: Vec::new(),
            });
            i
        });
        if threads[idx].contact.is_none() {
            threads[idx].contact = contact;
        }
        threads[idx].items.push(Mail {
            id,
            text,
            time,
            starred,
            locked,
            pgp,
            read,
            when,
        });
    }
    (threads, older)
}
