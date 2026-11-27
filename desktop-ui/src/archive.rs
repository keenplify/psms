#[path = "../../src/decrypt.rs"]
mod actions;
pub use actions::{decrypt, open_message};
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
    /// A reply sent from this computer.
    pub outgoing: bool,
    /// Messages-style time for the conversation list: "9:54 AM", "Yesterday", a weekday or a date.
    pub when: String,
    /// Who sent it, in a group chat.
    pub from: Option<String>,
    /// Files that came with it: (type, name, left out because it was too large).
    pub files: Vec<(String, String, bool)>,
    /// Tapbacks: (kind, who).
    pub reactions: Vec<(String, String)>,
    /// The message this one replies to, by archive ID.
    pub reply_to: Option<String>,
}
pub struct Thread {
    pub key: String,
    pub title: String,
    /// Hashed contact ID, when the sending shortcut provided one.
    pub contact: Option<String>,
    pub items: Vec<Mail>,
    /// A conversation from Messages: its opaque ID and sealed details (members, and the
    /// identifier pSMS for Mac sends to).
    pub chat: Option<(String, serde_json::Value)>,
}
/// A stored message as the inbox lists it: ID, how it's sealed, the sealed body, times, flags,
/// and the conversation, sealed reactions and file count it carries.
#[derive(Clone, Default)]
pub struct Record {
    pub id: String,
    pub provider: String,
    pub body: String,
    pub time: String,
    pub starred: bool,
    pub contact: Option<String>,
    pub read: bool,
    pub when: String,
    pub chat: Option<String>,
    pub reactions: Vec<String>,
    pub files: i64,
}
impl Record {
    /// A record from an inbox API listing (local service or papamo.dev).
    pub fn from_api(r: &serde_json::Value, time: String, when: String) -> Option<Record> {
        Some(Record {
            id: r["id"].as_str()?.into(),
            provider: r["sender"].as_str()?.into(),
            body: r["body"].as_str()?.into(),
            time,
            starred: r["starred"] == true,
            contact: r["contact_id"].as_str().map(String::from),
            read: r["read"] == true,
            when,
            chat: r["chat_id"].as_str().map(String::from),
            reactions: r["reactions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x["envelope"].as_str().map(String::from))
                .collect(),
            files: r["attachments"].as_i64().unwrap_or(0),
        })
    }
}
/// Emoji for a tapback, as Messages draws them.
pub fn reaction_emoji(kind: &str) -> &'static str {
    match kind {
        "love" => "❤️",
        "like" => "👍",
        "dislike" => "👎",
        "laugh" => "😂",
        "emphasize" => "‼️",
        "question" => "❓",
        _ => "•",
    }
}
/// The full time and the Messages-style label for a Unix time, as the local archive shows them.
pub fn times(db: &Connection, at: i64) -> Option<(String, String)> {
    db.query_row(
        "SELECT strftime('%Y-%m-%d %H:%M',?1,'unixepoch','localtime'), CASE WHEN date(?1,'unixepoch','localtime')=date('now','localtime') THEN ltrim(strftime('%I:%M %p',?1,'unixepoch','localtime'),'0')              WHEN date(?1,'unixepoch','localtime')=date('now','localtime','-1 day') THEN 'Yesterday'              WHEN date(?1,'unixepoch','localtime')>date('now','localtime','-7 days') THEN                CASE strftime('%w',?1,'unixepoch','localtime') WHEN '0' THEN 'Sunday' WHEN '1' THEN 'Monday' WHEN '2' THEN 'Tuesday' WHEN '3' THEN 'Wednesday' WHEN '4' THEN 'Thursday' WHEN '5' THEN 'Friday' ELSE 'Saturday' END              ELSE strftime('%m/%d/%y',?1,'unixepoch','localtime') END",
        [at],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .ok()
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
             ELSE strftime('%m/%d/%y',m.archived_at,'unixepoch','localtime') END,
        (SELECT chat_id FROM message_chats c WHERE c.message_id=m.id),
        (SELECT group_concat(envelope, ' ') FROM message_reactions x WHERE x.message_id=m.id),
        (SELECT count(*) FROM message_attachments a WHERE a.message_id=m.id)
        FROM messages m LEFT JOIN message_flags f ON m.id=f.message_id WHERE ?1=0 OR f.starred=1 ORDER BY m.rowid DESC LIMIT ?2 OFFSET ?3")?;
    let mut rows = stmt.query((starred, (LIMIT + 1) as i64, page * LIMIT as i64))?;
    let mut records = Vec::new();
    while let Some(row) = rows.next()? {
        records.push(Record {
            id: row.get(0)?,
            provider: row.get(1)?,
            body: row.get(2)?,
            time: row.get(3)?,
            starred: row.get(4)?,
            contact: row.get(5)?,
            read: row.get(6)?,
            when: row.get(7)?,
            chat: row.get(8).unwrap_or_default(),
            reactions: row
                .get::<_, Option<String>>(9)
                .unwrap_or_default()
                .map(|s| s.split(' ').map(String::from).collect())
                .unwrap_or_default(),
            files: row.get(10).unwrap_or_default(),
        });
    }
    Ok(group(records, key, &chats(db, key)))
}
/// Each chat's opened details, from the archive's `chats` table.
pub fn chats(db: &Connection, key: Option<&str>) -> HashMap<String, serde_json::Value> {
    let Some(key) = key else {
        return HashMap::new();
    };
    db.prepare("SELECT id, envelope FROM chats")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, envelope)| Some((id, crate::mac_messages::open_json(key, &envelope)?)))
        .collect()
}
/// Opens a `/chats` listing from an inbox API.
pub fn chats_from_api(list: &serde_json::Value, key: &str) -> HashMap<String, serde_json::Value> {
    list["chats"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some((
                c["id"].as_str()?.to_string(),
                crate::mac_messages::open_json(key, c["envelope"].as_str()?)?,
            ))
        })
        .collect()
}
/// Groups records into conversations: by chat when Messages provided one, otherwise by sender.
/// `chats` holds each chat's opened details from `/chats`.
pub fn group(
    records: Vec<Record>,
    key: Option<&str>,
    chats: &HashMap<String, serde_json::Value>,
) -> (Vec<Thread>, bool) {
    let older = records.len() > LIMIT;
    let mut threads: Vec<Thread> = Vec::new();
    let mut groups = HashMap::new();
    for r in records.into_iter().take(LIMIT) {
        let pgp = r.provider == "papamo:openpgp:v1";
        let sealed = r.provider == "papamo:encrypted:v1";
        let content = if sealed {
            key.and_then(|s| open_message(s, &r.body))
        } else if pgp {
            None
        } else {
            Some((r.provider.clone(), r.body.clone(), false))
        };
        let extra = sealed
            .then(|| key.and_then(|k| crate::mac_messages::open_json(k, &r.body)))
            .flatten()
            .unwrap_or_default();
        let locked = content.is_none();
        let (sender, text, outgoing) =
            content.unwrap_or_else(|| ("Encrypted message".into(), r.body.clone(), false));
        // Group chats get their own thread; one-to-one texts stay with the person, so copies
        // from Messages and from the iPhone Shortcut share one conversation.
        let chat = extra["thread"]
            .as_str()
            .map(String::from)
            .or_else(|| {
                r.chat
                    .clone()
                    .filter(|c| chats.get(c).is_some_and(|d| d["group"] == true))
            })
            .filter(|_| !locked);
        let group = match (&chat, locked) {
            (_, true) => format!("locked/{}", r.id),
            (Some(c), _) => format!("chat/{c}"),
            (None, _) => format!("sender/{sender}"),
        };
        let details = chat.as_ref().and_then(|c| chats.get(c)).cloned();
        let is_group = details.as_ref().is_some_and(|d| d["group"] == true);
        let idx = *groups.entry(group.clone()).or_insert_with(|| {
            let i = threads.len();
            let title = details
                .as_ref()
                .filter(|_| is_group)
                .and_then(|d| d["title"].as_str())
                .map(String::from)
                .unwrap_or_else(|| sender.clone());
            threads.push(Thread {
                key: group,
                title,
                contact: None,
                items: Vec::new(),
                chat: chat.clone().zip(details.clone()),
            });
            i
        });
        if threads[idx].contact.is_none() && !is_group {
            threads[idx].contact = r.contact.clone();
        }
        let reactions = r
            .reactions
            .iter()
            .filter_map(|e| key.and_then(|k| crate::mac_messages::open_json(k, e)))
            .filter_map(|v| {
                Some((
                    v["kind"].as_str()?.to_string(),
                    v["from"].as_str()?.to_string(),
                ))
            })
            .collect();
        let mut files: Vec<(String, String, bool)> = extra["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| {
                (
                    f["type"].as_str().unwrap_or_default().to_string(),
                    f["name"].as_str().unwrap_or("File").to_string(),
                    f.get("left_out").is_some(),
                )
            })
            .collect();
        // Files the record lists without details (from an older copy): show them generically.
        while (files.len() as i64) < r.files.min(10) {
            files.push(("application/octet-stream".into(), "File".into(), false));
        }
        threads[idx].items.push(Mail {
            id: r.id,
            text,
            time: r.time,
            starred: r.starred,
            locked,
            pgp,
            read: r.read,
            when: r.when,
            outgoing,
            from: (is_group && !outgoing).then(|| sender.clone()),
            files,
            reactions,
            reply_to: extra["reply_to"].as_str().map(String::from),
        });
    }
    (threads, older)
}
