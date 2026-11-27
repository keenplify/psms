//! Messages on this Mac, read the way BlueBubbles reads them: pSMS watches the Messages
//! database (`~/Library/Messages/chat.db`, which needs Full Disk Access) for new rows and stores
//! each one sealed in the archive. That brings what the iPhone Shortcut can't see: photos and
//! other files, group chats and their members, texts you sent from your phone, tapback
//! reactions and threaded replies. It is off until turned on, and starts from that moment.
//!
//! Each Messages row becomes one sealed record. Reactions and replies point at the record they
//! belong to by its archive ID, which is derived from the Messages GUID and the archive key, so
//! running the same row twice stores it once and the GUID never leaves this Mac.

use crate::{api::Api, relay::Relay, replies};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What to copy from Messages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    /// Photos, files and group chats: what the iPhone Shortcut can't send.
    Media,
    /// Every iMessage and text, both directions, like BlueBubbles.
    All,
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS desktop_mac_messages(id INTEGER PRIMARY KEY CHECK(id=1), mode TEXT NOT NULL, last_rowid INTEGER NOT NULL, copied INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS desktop_mac_chats(chat_id TEXT PRIMARY KEY, digest TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS desktop_mac_edits(id INTEGER PRIMARY KEY CHECK(id=1), after INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS desktop_capture_watch(guid TEXT PRIMARY KEY, party TEXT NOT NULL, text TEXT NOT NULL, at INTEGER NOT NULL, resolved INTEGER NOT NULL DEFAULT 0, reported INTEGER NOT NULL DEFAULT 0)";
/// How long the iPhone Shortcut gets to send a text the Mac already has before pSMS warns.
pub const CAPTURE_GRACE_SECS: i64 = 600;
/// The largest file copied, before sealing; larger ones are listed but not copied.
pub const MAX_FILE: usize = 1_500_000;
/// Wait this long after a text arrives, so the iPhone Shortcut's copy is in first.
const SETTLE_SECS: i64 = 15;
/// Wait up to this long for Messages to finish downloading a photo.
const DOWNLOAD_SECS: i64 = 180;

pub fn mode(db: &Connection) -> Mode {
    match db
        .query_row(
            "SELECT mode FROM desktop_mac_messages WHERE id=1",
            [],
            |r| r.get::<_, String>(0),
        )
        .as_deref()
    {
        Ok("media") => Mode::Media,
        Ok("all") => Mode::All,
        _ => Mode::Off,
    }
}
/// How many messages this Mac has copied since it was turned on.
pub fn copied(db: &Connection) -> i64 {
    db.query_row(
        "SELECT copied FROM desktop_mac_messages WHERE id=1",
        [],
        |r| r.get(0),
    )
    .unwrap_or(0)
}
/// Turns copying on or off. Turning it on starts from the newest message, so history stays
/// where it is.
pub fn set_mode(db: &Connection, mode: Mode) -> Result<(), String> {
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    let name = match mode {
        Mode::Off => "off",
        Mode::Media => "media",
        Mode::All => "all",
    };
    let start = if mode == Mode::Off {
        0
    } else {
        let chat = open_chat_db().map_err(|_| access_hint())?;
        chat.query_row("SELECT coalesce(max(ROWID),0) FROM message", [], |r| {
            r.get(0)
        })
        .map_err(|_| access_hint())?
    };
    // Edits and unsends count from now too.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() as i64 - 978_307_200) * 1_000_000_000)
        .unwrap_or_default();
    db.execute(
        "INSERT INTO desktop_mac_edits(id,after) VALUES(1,?1) ON CONFLICT(id) DO NOTHING",
        [now],
    )
    .map_err(|e| e.to_string())?;
    db.execute(
        "INSERT INTO desktop_mac_messages(id,mode,last_rowid) VALUES(1,?1,?2)
         ON CONFLICT(id) DO UPDATE SET mode=excluded.mode,
           last_rowid=CASE WHEN desktop_mac_messages.mode='off' OR excluded.mode='off' THEN excluded.last_rowid ELSE desktop_mac_messages.last_rowid END",
        params![name, start],
    )
    .map(drop)
    .map_err(|e| e.to_string())
}
pub fn access_hint() -> String {
    "pSMS needs Full Disk Access to read Messages. Open System Settings → Privacy & Security → Full Disk Access, turn on pSMS, then try again.".into()
}

pub fn chat_db_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PSMS_CHAT_DB") {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Messages/chat.db"))
}
fn open_chat_db() -> rusqlite::Result<Connection> {
    let path = chat_db_path().ok_or(rusqlite::Error::InvalidQuery)?;
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.query_row("SELECT 1 FROM message LIMIT 1", [], |_| Ok(()))
        .optional()?;
    Ok(db)
}
/// Whether this app can read Messages, which macOS allows only with Full Disk Access.
pub fn readable() -> bool {
    open_chat_db().is_ok()
}

/// One file that came with a message.
#[derive(Debug, Clone)]
pub struct File {
    pub path: PathBuf,
    pub mime: String,
    pub name: String,
    pub size: i64,
    pub sticker: bool,
}
/// One Messages row, ready to seal.
#[derive(Debug, Clone)]
pub struct Item {
    pub rowid: i64,
    pub guid: String,
    pub text: String,
    pub from_me: bool,
    /// Who sent it: their phone number or email. Empty for your own messages.
    pub sender: String,
    pub at: i64,
    pub service: String,
    pub chat_guid: String,
    pub chat_identifier: String,
    pub chat_name: String,
    pub group: bool,
    pub members: Vec<String>,
    pub files: Vec<File>,
    /// A tapback: its kind, the GUID it reacts to, and whether it removes one.
    pub reaction: Option<(String, String, bool)>,
    pub reply_to: Option<String>,
}

/// Seconds since 1970 from a Messages date (nanoseconds since 2001, or seconds on old Macs).
pub fn unix_time(apple: i64) -> i64 {
    let seconds = if apple > 1_000_000_000_000 {
        apple / 1_000_000_000
    } else {
        apple
    };
    seconds + 978_307_200
}

/// The text of a message whose `text` column is empty, from its `attributedBody` archive.
/// Newer macOS versions store most texts this way. The string follows the `NSString` class
/// name, as a `+` byte and a length: one byte, or 0x81 and two bytes, or 0x82 and four bytes,
/// little-endian.
pub fn attributed_text(blob: &[u8]) -> Option<String> {
    let start = blob.windows(8).position(|w| w == b"NSString")? + 8;
    let plus = start
        + blob
            .get(start..start + 12)?
            .iter()
            .position(|&b| b == b'+')?
        + 1;
    let (len, from) = match *blob.get(plus)? {
        0x81 => (
            u16::from_le_bytes([*blob.get(plus + 1)?, *blob.get(plus + 2)?]) as usize,
            plus + 3,
        ),
        0x82 => (
            u32::from_le_bytes(blob.get(plus + 1..plus + 5)?.try_into().ok()?) as usize,
            plus + 5,
        ),
        n => (n as usize, plus + 1),
    };
    String::from_utf8(blob.get(from..from + len)?.to_vec()).ok()
}

const REACTIONS: [&str; 6] = ["love", "like", "dislike", "laugh", "emphasize", "question"];
/// A tapback from `associated_message_type`: 2000-2005 add one, 3000-3005 remove it.
fn reaction(kind: i64, target: Option<String>) -> Option<(String, String, bool)> {
    let target = target?;
    // Targets look like "p:0/GUID" (a part of a message) or "bp:GUID".
    let guid = target.rsplit(['/', ':']).next()?.to_string();
    let (base, removed) = match kind {
        2000..=2005 => (kind - 2000, false),
        3000..=3005 => (kind - 3000, true),
        _ => return None,
    };
    Some((REACTIONS[base as usize].to_string(), guid, removed))
}

/// New rows after `after`, oldest first.
pub fn read_new(chat: &Connection, after: i64, limit: i64) -> rusqlite::Result<Vec<Item>> {
    let mut stmt = chat.prepare(
        "SELECT m.ROWID, m.guid, m.text, m.attributedBody, m.is_from_me, coalesce(h.id,''), m.date,
                coalesce(m.service,''), coalesce(c.guid,''), coalesce(c.chat_identifier,''),
                coalesce(c.display_name,''), coalesce(c.style,45), m.associated_message_type,
                m.associated_message_guid, m.thread_originator_guid
         FROM message m
         LEFT JOIN handle h ON h.ROWID = m.handle_id
         LEFT JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
         LEFT JOIN chat c ON c.ROWID = cmj.chat_id
         WHERE m.ROWID > ?1 AND coalesce(m.item_type,0) = 0
         GROUP BY m.ROWID ORDER BY m.ROWID LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![after, limit], |r| {
            let text: Option<String> = r.get(2)?;
            let body: Option<Vec<u8>> = r.get(3)?;
            Ok(Item {
                rowid: r.get(0)?,
                guid: r.get(1)?,
                text: text
                    .filter(|t| !t.is_empty())
                    .or_else(|| body.as_deref().and_then(attributed_text))
                    .unwrap_or_default()
                    .replace('\u{fffc}', "")
                    .trim()
                    .to_string(),
                from_me: r.get::<_, i64>(4)? != 0,
                sender: r.get(5)?,
                at: unix_time(r.get::<_, Option<i64>>(6)?.unwrap_or_default()),
                service: r.get(7)?,
                chat_guid: r.get(8)?,
                chat_identifier: r.get(9)?,
                chat_name: r.get(10)?,
                group: r.get::<_, i64>(11)? == 43,
                members: Vec::new(),
                files: Vec::new(),
                reaction: reaction(r.get::<_, Option<i64>>(12)?.unwrap_or_default(), r.get(13)?),
                reply_to: r.get::<_, Option<String>>(14)?.filter(|g| !g.is_empty()),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut items = Vec::with_capacity(rows.len());
    for mut item in rows {
        if item.group {
            item.members = chat
                .prepare_cached(
                    "SELECT h.id FROM chat_handle_join j JOIN handle h ON h.ROWID=j.handle_id
                     JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=?1 ORDER BY h.id",
                )?
                .query_map([&item.chat_guid], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
        }
        item.files = chat
            .prepare_cached(
                "SELECT coalesce(a.filename,''), coalesce(a.mime_type,''), coalesce(a.transfer_name,''),
                        coalesce(a.total_bytes,0), coalesce(a.is_sticker,0)
                 FROM message_attachment_join j JOIN attachment a ON a.ROWID=j.attachment_id
                 WHERE j.message_id=?1 ORDER BY a.ROWID",
            )?
            .query_map([item.rowid], |r| {
                let raw: String = r.get(0)?;
                Ok(File {
                    path: expand(&raw),
                    mime: r.get(1)?,
                    name: r.get(2)?,
                    size: r.get(3)?,
                    sticker: r.get::<_, i64>(4)? != 0,
                })
            })?
            .filter_map(Result::ok)
            .filter(|f| !f.path.as_os_str().is_empty())
            .collect();
        items.push(item);
    }
    Ok(items)
}
fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

/// A message edited or unsent after it was copied: its GUID, its text now, whether it was
/// unsent, and when (a Messages date).
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub guid: String,
    pub text: String,
    pub unsent: bool,
    pub at: i64,
}
/// Rows up to `upto` that were edited or unsent after `after`, oldest change first. Like
/// BlueBubbles, an unsent message is one with a retraction date or, once edited, no text left.
/// Macs before Ventura have no edit columns; then there is nothing to read.
pub fn read_changes(chat: &Connection, after: i64, upto: i64) -> rusqlite::Result<Vec<Change>> {
    let mut stmt = chat.prepare(
        "SELECT guid, text, attributedBody, coalesce(date_edited,0), coalesce(date_retracted,0)
         FROM message WHERE ROWID <= ?2
           AND (coalesce(date_edited,0) > ?1 OR coalesce(date_retracted,0) > ?1)
         ORDER BY max(coalesce(date_edited,0), coalesce(date_retracted,0)) LIMIT 50",
    )?;
    stmt.query_map(params![after, upto], |r| {
        let text: Option<String> = r.get(1)?;
        let body: Option<Vec<u8>> = r.get(2)?;
        let text = text
            .filter(|t| !t.is_empty())
            .or_else(|| body.as_deref().and_then(attributed_text))
            .unwrap_or_default()
            .replace('\u{fffc}', "")
            .trim()
            .to_string();
        let (edited, retracted): (i64, i64) = (r.get(3)?, r.get(4)?);
        Ok(Change {
            guid: r.get(0)?,
            unsent: retracted > 0 || text.is_empty(),
            text,
            at: edited.max(retracted),
        })
    })?
    .collect()
}
/// Seals edits and unsends onto the messages they change, as one replaceable event each.
fn copy_changes(
    db: &Connection,
    chat: &Connection,
    key: &str,
    inbox: &Inbox,
    upto: i64,
) -> Result<usize, String> {
    let Ok(after) = db.query_row("SELECT after FROM desktop_mac_edits WHERE id=1", [], |r| {
        r.get::<_, i64>(0)
    }) else {
        return Ok(0);
    };
    let Ok(changes) = read_changes(chat, after, upto) else {
        return Ok(0);
    };
    let mut newest = after;
    let mut stored = 0;
    for change in changes {
        let event = json!({
            "kind": if change.unsent { "unsend" } else { "edit" },
            "from": "",
            "text": if change.unsent { "" } else { change.text.as_str() },
            "at": unix_time(change.at),
        });
        let sealed = seal_json(key, &event).ok_or("Could not seal an edit.")?;
        let route = format!(
            "/messages/{}/reactions/{}",
            record_id(&change.guid, key),
            record_id(&format!("{}\nchange", change.guid), key)
        );
        // A change to a message this inbox doesn't have is skipped.
        if inbox
            .call(
                reqwest::Method::PUT,
                &route,
                Some(json!({"envelope": sealed})),
            )
            .is_ok()
        {
            stored += 1;
        }
        newest = newest.max(change.at);
    }
    db.execute("UPDATE desktop_mac_edits SET after=?1 WHERE id=1", [newest])
        .map_err(|e| e.to_string())?;
    Ok(stored)
}
/// Texts the Mac saw that the iPhone Shortcut hasn't sent after the grace period. Each is
/// reported once; returns how many were newly missed. Only inboxes a phone sends to are
/// checked, so a Mac-only setup never warns.
pub fn check_capture(db: &Connection, key: &str, inbox: &Inbox) -> Result<usize, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let waiting: i64 = db
        .query_row(
            "SELECT count(*) FROM desktop_capture_watch WHERE NOT resolved AND NOT reported AND at > ?1",
            [now - 86_400],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if waiting == 0 {
        return Ok(0);
    }
    let health = inbox.call(reqwest::Method::GET, "/health", None)?;
    let has_phone = health["phones"]
        .as_array()
        .is_some_and(|p| p.iter().any(|p| !p["last_message_at"].is_null()));
    if !has_phone {
        let _ = db.execute(
            "UPDATE desktop_capture_watch SET resolved=1 WHERE NOT resolved",
            [],
        );
        return Ok(0);
    }
    let recent: Vec<(String, Value, i64)> =
        inbox.call(reqwest::Method::GET, "/messages?limit=60", None)?["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|m| {
                Some((
                    m["id"].as_str()?.to_string(),
                    open_json(key, m["body"].as_str()?)?,
                    m["received_at"].as_i64().or(m["archived_at"].as_i64())?,
                ))
            })
            .collect();
    let watched: Vec<(String, String, String, i64)> = db
        .prepare("SELECT guid,party,text,at FROM desktop_capture_watch WHERE NOT resolved AND NOT reported AND at > ?1")
        .and_then(|mut stmt| {
            stmt.query_map([now - 86_400], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect()
        })
        .map_err(|e| e.to_string())?;
    let mut missed = 0;
    for (guid, party, text, at) in watched {
        let probe = Item {
            rowid: 0,
            guid: guid.clone(),
            text,
            from_me: false,
            sender: party.clone(),
            at,
            service: String::new(),
            chat_guid: String::new(),
            chat_identifier: party,
            chat_name: String::new(),
            group: false,
            members: vec![],
            files: vec![],
            reaction: None,
            reply_to: None,
        };
        if duplicate(&recent, &probe).is_some() {
            let _ = db.execute(
                "UPDATE desktop_capture_watch SET resolved=1 WHERE guid=?1",
                [&guid],
            );
        } else if now - at > CAPTURE_GRACE_SECS {
            let _ = db.execute(
                "UPDATE desktop_capture_watch SET reported=1 WHERE guid=?1",
                [&guid],
            );
            missed += 1;
        }
    }
    let _ = db.execute(
        "DELETE FROM desktop_capture_watch WHERE at < ?1",
        [now - 7 * 86_400],
    );
    if missed > 0 {
        let _ = inbox.call(reqwest::Method::POST, "/health/missed", None);
    }
    Ok(missed)
}
/// Whether a mode copies this row.
pub fn wanted(mode: Mode, item: &Item) -> bool {
    match mode {
        Mode::Off => false,
        Mode::All => !item.text.is_empty() || !item.files.is_empty() || item.reaction.is_some(),
        Mode::Media => {
            (item.group
                && (!item.text.is_empty() || !item.files.is_empty() || item.reaction.is_some()))
                || !item.files.is_empty()
        }
    }
}

/// The archive ID for a Messages GUID: stable for this archive, and not the GUID itself.
pub fn record_id(guid: &str, key: &str) -> String {
    let digest = Sha256::digest(format!("psms-messages-v1\n{guid}\n{key}"));
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string()
}
/// The opaque ID a chat has on the server, shared by its messages.
pub fn chat_id(chat_guid: &str, key: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("psms-chat-v1\n{chat_guid}\n{key}"))
    )[..32]
        .to_string()
}
/// A chat's sealed details: what clients show, and what pSMS for Mac sends to.
pub fn chat_record(item: &Item) -> Value {
    json!({
        "title": if item.group { group_title(item) } else { item.chat_identifier.clone() },
        "name": item.chat_name,
        "group": item.group,
        "members": if item.group { item.members.clone() } else { vec![item.chat_identifier.clone()] },
        "service": item.service,
        "identifier": item.chat_guid,
    })
}
/// The ID of one person's tapback of one kind on one message, so removing it finds it.
pub fn reaction_id(target_guid: &str, from: &str, kind: &str, key: &str) -> String {
    record_id(&format!("{target_guid}\n{from}\n{kind}"), key)
}
/// Who a record belongs to in the conversation list: the other person in a one-to-one chat.
fn party(item: &Item) -> String {
    if item.from_me || item.sender.is_empty() {
        item.chat_identifier.clone()
    } else {
        item.sender.clone()
    }
}
/// The sealed record's contents. Every client reads the same fields: `thread` groups a chat,
/// `group` names it, `attachments` lists files by index, and `reaction`/`reply_to` point at
/// other records by archive ID.
pub fn record(item: &Item, key: &str, files: &[Value]) -> Value {
    let mut value = json!({
        "sender": party(item),
        "body": item.text,
        "source": "messages",
        "service": item.service,
        "sent_at": item.at,
    });
    if item.from_me {
        value["direction"] = json!("out");
    }
    if item.group {
        // Older clients group by sender; the chat ID is what groups it now.
        value["thread"] = json!(chat_id(&item.chat_guid, key));
        if item.from_me {
            value["sender"] = json!(group_title(item));
        }
    }
    if !files.is_empty() {
        value["attachments"] = json!(files);
    }
    if let Some(target) = &item.reply_to {
        value["reply_to"] = json!(record_id(target, key));
    }
    value
}
/// A group's name, or its members when it has none, as Messages shows it.
pub fn group_title(item: &Item) -> String {
    if !item.chat_name.is_empty() {
        return item.chat_name.clone();
    }
    let names = item.members.join(", ");
    if names.is_empty() {
        "Group".into()
    } else {
        names
    }
}

/// A file ready to seal: photos are shrunk and HEIC becomes JPEG, as BlueBubbles converts
/// them for clients. Returns the bytes and their type, or why it was left out.
pub fn prepare(file: &File) -> Result<(Vec<u8>, String), &'static str> {
    let bytes = std::fs::read(&file.path).map_err(|_| "missing")?;
    let mime = if file.mime.is_empty() {
        guess(&file.name)
    } else {
        file.mime.clone()
    };
    let image = mime.starts_with("image/");
    let fits = bytes.len() <= MAX_FILE;
    let web_ready = matches!(
        mime.as_str(),
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    );
    if !image || (fits && web_ready) {
        return if fits {
            Ok((bytes, mime))
        } else {
            Err("too large")
        };
    }
    // HEIC, TIFF and large photos: a JPEG up to 2048 pixels, made with macOS's own sips.
    let out = std::env::temp_dir().join(format!("psms-photo-{}.jpg", uuid::Uuid::new_v4()));
    let ok = std::process::Command::new("/usr/bin/sips")
        .args([
            "-s",
            "format",
            "jpeg",
            "-s",
            "formatOptions",
            "80",
            "-Z",
            "2048",
        ])
        .arg(&file.path)
        .arg("--out")
        .arg(&out)
        .output()
        .is_ok_and(|o| o.status.success());
    let converted = std::fs::read(&out).ok().filter(|_| ok);
    let _ = std::fs::remove_file(&out);
    match converted {
        Some(jpeg) if jpeg.len() <= MAX_FILE => Ok((jpeg, "image/jpeg".into())),
        _ => Err("too large"),
    }
}
fn guess(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "heic" | "heif" => "image/heic",
        "webp" => "image/webp",
        "mov" => "video/quicktime",
        "mp4" => "video/mp4",
        "m4a" | "caf" => "audio/mp4",
        "pdf" => "application/pdf",
        "vcf" => "text/vcard",
        _ => "application/octet-stream",
    }
    .into()
}
/// Seals file bytes with the archive key, in the same envelope as messages.
pub fn seal_bytes(key: &str, bytes: &[u8]) -> Option<String> {
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).ok()?;
    let sealed = Aes256Gcm::new_from_slice(key.as_bytes())
        .ok()?
        .encrypt(Nonce::from_slice(&nonce), bytes)
        .ok()?;
    Some(STANDARD.encode([nonce.as_slice(), &sealed].concat()))
}
pub fn seal_json(key: &str, value: &Value) -> Option<String> {
    seal_bytes(key, value.to_string().as_bytes())
}
/// Opens a sealed file.
pub fn open_bytes(key: &str, envelope: &str) -> Option<Vec<u8>> {
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
    let bytes = STANDARD.decode(envelope).ok()?;
    if bytes.len() <= 28 {
        return None;
    }
    Aes256Gcm::new_from_slice(key.as_bytes())
        .ok()?
        .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
        .ok()
}
pub fn open_json(key: &str, envelope: &str) -> Option<Value> {
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
    let bytes = STANDARD.decode(envelope).ok()?;
    if bytes.len() <= 28 {
        return None;
    }
    let clear = Aes256Gcm::new_from_slice(key.as_bytes())
        .ok()?
        .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
        .ok()?;
    serde_json::from_slice(&clear).ok()
}

/// Where records go: a papamo.dev inbox through the relay, or this computer's archive.
#[derive(Clone)]
pub enum Inbox {
    Hosted(Arc<Relay>),
    Local(Api),
}
impl Inbox {
    /// A file that came with a message, opened with the archive key.
    pub fn attachment(&self, key: &str, message: &str, index: usize) -> Result<Vec<u8>, String> {
        let v = self.call(
            reqwest::Method::GET,
            &format!("/messages/{message}/attachments/{index}"),
            None,
        )?;
        open_bytes(key, v["envelope"].as_str().unwrap_or_default())
            .ok_or_else(|| "Could not open this file with your key.".to_string())
    }
    pub(crate) fn call(
        &self,
        method: reqwest::Method,
        route: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        match self {
            Inbox::Hosted(r) => r.call(method, route, body),
            Inbox::Local(a) => a
                .call(method, route, body)
                .map_err(|_| "The archive service did not accept it.".to_string()),
        }
    }
}

fn digits(s: &str) -> String {
    let d: String = s.chars().filter(char::is_ascii_digit).collect();
    d[d.len().saturating_sub(10)..].to_string()
}
fn same_party(a: &str, b: &str) -> bool {
    let (da, db) = (digits(a), digits(b));
    if da.len() >= 7 && db.len() >= 7 {
        da == db
    } else {
        a.eq_ignore_ascii_case(b)
    }
}

/// A copy of this row that came another way, such as the iPhone Shortcut or a reply pSMS
/// sent: (its archive ID, whether it is your own message).
fn duplicate(recent: &[(String, Value, i64)], item: &Item) -> Option<(String, bool)> {
    if item.text.is_empty() || item.group {
        return None;
    }
    recent.iter().find_map(|(id, v, at)| {
        let other_source = v["source"].as_str() != Some("messages");
        let out = v["direction"].as_str() == Some("out");
        (other_source
            && out == item.from_me
            && v["body"].as_str().map(str::trim) == Some(item.text.as_str())
            && same_party(v["sender"].as_str().unwrap_or_default(), &party(item))
            && (at - item.at).abs() < 600)
            .then(|| (id.clone(), out))
    })
}

/// Copies new Messages rows into the inbox. Returns how many records it stored. Rows wait
/// until they have settled and their files have downloaded; the rest wait for the next run.
pub fn sync(db: &Connection, key: &str, inbox: &Inbox) -> Result<usize, String> {
    let mode = mode(db);
    if mode == Mode::Off {
        return Ok(0);
    }
    let after: i64 = db
        .query_row(
            "SELECT last_rowid FROM desktop_mac_messages WHERE id=1",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let chat = open_chat_db().map_err(|_| access_hint())?;
    let items = read_new(&chat, after, 50).map_err(|e| e.to_string())?;
    if items.is_empty() {
        return copy_changes(db, &chat, key, inbox, after);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let recent: Vec<(String, Value, i64)> = inbox
        .call(reqwest::Method::GET, "/messages?limit=40", None)
        .ok()
        .and_then(|v| v["messages"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| {
            Some((
                m["id"].as_str()?.to_string(),
                open_json(key, m["body"].as_str()?)?,
                m["received_at"].as_i64().or(m["archived_at"].as_i64())?,
            ))
        })
        .collect();
    let mut stored = 0;
    let mut last = after;
    for item in items {
        if now - item.at < SETTLE_SECS {
            break;
        }
        // Watch every incoming one-to-one text: the iPhone Shortcut should send it too.
        if !item.from_me && !item.group && !item.text.is_empty() && item.reaction.is_none() {
            let _ = db.execute(
                "INSERT OR IGNORE INTO desktop_capture_watch(guid,party,text,at,resolved) VALUES(?1,?2,?3,?4,?5)",
                params![item.guid, party(&item), item.text, item.at, duplicate(&recent, &item).is_some()],
            );
        }
        if !wanted(mode, &item) {
            last = item.rowid;
            continue;
        }
        let mut files = Vec::new();
        let mut sealed = Vec::new();
        let mut waiting = false;
        for file in &item.files {
            match prepare(file) {
                Ok((bytes, mime)) => {
                    files.push(json!({"type": mime, "name": file.name, "size": bytes.len(), "sticker": file.sticker}));
                    sealed.push(seal_bytes(key, &bytes).ok_or("Could not seal a file.")?);
                }
                Err("missing") if now - item.at < DOWNLOAD_SECS => {
                    waiting = true;
                    break;
                }
                Err(why) => files.push(json!({"type": file.mime, "name": file.name, "size": file.size, "left_out": why})),
            }
        }
        if waiting {
            break;
        }
        if let Some((kind, target, removed)) = &item.reaction {
            // A tapback hangs off the message it reacts to; removing one deletes it.
            let from = if item.from_me {
                "me"
            } else {
                item.sender.as_str()
            };
            let route = format!(
                "/messages/{}/reactions/{}",
                record_id(target, key),
                reaction_id(target, from, kind, key)
            );
            let result = if *removed {
                inbox.call(reqwest::Method::DELETE, &route, None)
            } else {
                let sealed = seal_json(key, &json!({"kind": kind, "from": from, "at": item.at}))
                    .ok_or("Could not seal a reaction.")?;
                inbox.call(
                    reqwest::Method::PUT,
                    &route,
                    Some(json!({"envelope": sealed})),
                )
            };
            // A reaction to a message this inbox doesn't have is skipped.
            let _ = result;
            stored += 1;
            last = item.rowid;
            continue;
        }
        if !item.chat_guid.is_empty() {
            let chat = chat_id(&item.chat_guid, key);
            let details = chat_record(&item);
            let digest = format!("{:x}", Sha256::digest(details.to_string()));
            let known: Option<String> = db
                .query_row(
                    "SELECT digest FROM desktop_mac_chats WHERE chat_id=?1",
                    [&chat],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if known.as_deref() != Some(digest.as_str()) {
                let sealed = seal_json(key, &details).ok_or("Could not seal a chat.")?;
                inbox.call(
                    reqwest::Method::PUT,
                    &format!("/chats/{chat}"),
                    Some(json!({"envelope": sealed})),
                )?;
                db.execute(
                    "INSERT INTO desktop_mac_chats(chat_id,digest) VALUES(?1,?2) ON CONFLICT(chat_id) DO UPDATE SET digest=excluded.digest",
                    params![chat, digest],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        let dup = duplicate(&recent, &item);
        if let Some((_, true)) = dup {
            // pSMS already stored this reply when it sent it.
            last = item.rowid;
            continue;
        }
        let id = record_id(&item.guid, key);
        let body =
            seal_json(key, &record(&item, key, &files)).ok_or("Could not seal a message.")?;
        let contact =
            (!item.group || !item.from_me).then(|| replies::contact_id(&party(&item), key));
        inbox.call(
            reqwest::Method::POST,
            "/messages",
            Some(json!({
                "id": id, "sender": "papamo:encrypted:v1", "body": body, "contact_id": contact,
                "chat_id": (!item.chat_guid.is_empty()).then(|| chat_id(&item.chat_guid, key)),
            })),
        )?;
        for (index, envelope) in sealed.into_iter().enumerate() {
            let index = index.min(9);
            inbox.call(
                reqwest::Method::PUT,
                &format!("/messages/{id}/attachments/{index}"),
                Some(json!({"envelope": envelope})),
            )?;
        }
        if item.from_me {
            let _ = inbox.call(
                reqwest::Method::POST,
                "/messages/read",
                Some(json!({"ids": [id]})),
            );
        }
        // The Shortcut's copy of the same text: this richer one replaces it.
        if let Some((other, false)) = dup {
            let _ = inbox.call(reqwest::Method::DELETE, &format!("/messages/{other}"), None);
        }
        stored += 1;
        last = item.rowid;
    }
    db.execute(
        "UPDATE desktop_mac_messages SET last_rowid=?1, copied=copied+?2 WHERE id=1",
        params![last, stored as i64],
    )
    .map_err(|e| e.to_string())?;
    Ok(stored + copy_changes(db, &chat, key, inbox, last)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Messages database with the columns pSMS reads, like BlueBubbles' test fixtures.
    pub fn fixture(path: &Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.execute_batch(
            "CREATE TABLE message(ROWID INTEGER PRIMARY KEY, guid TEXT, text TEXT, attributedBody BLOB,
               handle_id INTEGER, service TEXT, date INTEGER, is_from_me INTEGER, item_type INTEGER,
               associated_message_type INTEGER, associated_message_guid TEXT, thread_originator_guid TEXT,
               date_edited INTEGER, date_retracted INTEGER);
             CREATE TABLE handle(ROWID INTEGER PRIMARY KEY, id TEXT);
             CREATE TABLE chat(ROWID INTEGER PRIMARY KEY, guid TEXT, chat_identifier TEXT, display_name TEXT, style INTEGER);
             CREATE TABLE chat_message_join(chat_id INTEGER, message_id INTEGER);
             CREATE TABLE chat_handle_join(chat_id INTEGER, handle_id INTEGER);
             CREATE TABLE attachment(ROWID INTEGER PRIMARY KEY, filename TEXT, mime_type TEXT, transfer_name TEXT, total_bytes INTEGER, is_sticker INTEGER);
             CREATE TABLE message_attachment_join(message_id INTEGER, attachment_id INTEGER);
             INSERT INTO handle VALUES(1,'+15550142'),(2,'+15550187');
             INSERT INTO chat VALUES(1,'iMessage;-;+15550142','+15550142','',45),
                                    (2,'iMessage;+;chat777','chat777','Lunch crew',43);
             INSERT INTO chat_handle_join VALUES(2,1),(2,2);",
        )
        .unwrap();
        db
    }

    #[test]
    fn reads_attributed_bodies() {
        let mut blob = b"\x04\x0bstreamtyped\x81\xe8\x03\x84\x01@\x84\x84\x84\x12NSAttributedString\x00\x84\x84\x08NSObject\x00\x85\x92\x84\x84\x84\x08NSString\x01\x94\x84\x01+".to_vec();
        blob.push(5);
        blob.extend_from_slice(b"Hello\x86\x84");
        assert_eq!(attributed_text(&blob).as_deref(), Some("Hello"));
        let long = "x".repeat(300);
        let mut blob = b"..NSString\x01\x94\x84\x01+\x81".to_vec();
        blob.extend_from_slice(&(300u16).to_le_bytes());
        blob.extend_from_slice(long.as_bytes());
        assert_eq!(attributed_text(&blob), Some(long));
        assert_eq!(attributed_text(b"nothing here"), None);
    }

    #[test]
    fn reads_texts_groups_files_reactions_and_replies() {
        let dir = std::env::temp_dir().join(format!("psms-chatdb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let photo = dir.join("IMG_1.png");
        std::fs::write(&photo, b"\x89PNG fake").unwrap();
        let db = fixture(&dir.join("chat.db"));
        let date = (1_795_784_000i64 - 978_307_200) * 1_000_000_000;
        let mut body = b"..NSString\x01\x94\x84\x01+".to_vec();
        body.push(13);
        body.extend_from_slice(b"From the body");
        db.execute_batch(&format!(
            "INSERT INTO message VALUES(10,'G-A','Lunch at 12?',NULL,1,'iMessage',{date},0,0,0,NULL,NULL,NULL,NULL);
             INSERT INTO message VALUES(11,'G-B',NULL,NULL,2,'iMessage',{date},0,0,0,NULL,'G-A',NULL,NULL);
             INSERT INTO message VALUES(12,'G-C',NULL,NULL,0,'iMessage',{date},1,0,2001,'p:0/G-A',NULL,NULL,NULL);
             INSERT INTO message VALUES(13,'G-D','renamed',NULL,0,'iMessage',{date},0,2,0,NULL,NULL,NULL,NULL);
             INSERT INTO message VALUES(14,'G-E','Hi',NULL,1,'SMS',{date},0,0,0,NULL,NULL,NULL,NULL);
             INSERT INTO chat_message_join VALUES(2,10),(2,11),(2,12),(2,13),(1,14);
             INSERT INTO attachment VALUES(1,'{}','image/png','IMG_1.png',10,0);
             INSERT INTO message_attachment_join VALUES(11,1);",
            photo.display()
        ))
        .unwrap();
        db.execute(
            "UPDATE message SET attributedBody=?1 WHERE ROWID=11",
            [body],
        )
        .unwrap();
        let items = read_new(&db, 0, 50).unwrap();
        // The group rename (item_type 2) is not a message.
        assert_eq!(
            items.iter().map(|i| i.rowid).collect::<Vec<_>>(),
            [10, 11, 12, 14]
        );
        let (a, b, c, d) = (&items[0], &items[1], &items[2], &items[3]);
        assert!(a.group && a.chat_name == "Lunch crew" && a.members == ["+15550142", "+15550187"]);
        assert_eq!(a.at, 1_795_784_000);
        assert_eq!(b.text, "From the body");
        assert_eq!(b.reply_to.as_deref(), Some("G-A"));
        assert_eq!(b.files[0].path, photo);
        assert_eq!(c.reaction, Some(("like".into(), "G-A".into(), false)));
        assert!(c.from_me && !d.group && d.service == "SMS");
        // What each mode copies: Media skips the plain one-to-one text.
        assert!(wanted(Mode::Media, a) && wanted(Mode::Media, b) && !wanted(Mode::Media, d));
        assert!(wanted(Mode::All, d));
        // Records point at each other by archive ID, never by Messages GUID.
        let key = "k".repeat(32);
        let r = record(b, &key, &[json!({"type":"image/png"})]);
        assert_eq!(r["reply_to"], json!(record_id("G-A", &key)));
        assert_eq!(r["thread"], json!(chat_id(&a.chat_guid, &key)));
        assert_eq!(r["sender"], "+15550187");
        assert_eq!(record(c, &key, &[])["sender"], "Lunch crew");
        assert_eq!(chat_id(&a.chat_guid, &key).len(), 32);
        let details = chat_record(a);
        assert_eq!(
            (details["title"].as_str(), details["identifier"].as_str()),
            (Some("Lunch crew"), Some("iMessage;+;chat777"))
        );
        assert_ne!(
            reaction_id("G-A", "me", "like", &key),
            reaction_id("G-A", "me", "love", &key)
        );
        assert!(!r.to_string().contains("G-A"));
        assert_eq!(record_id("G-A", &key), record_id("G-A", &key));
        assert_ne!(record_id("G-A", &key), record_id("G-A", &"j".repeat(32)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_edits_and_unsends() {
        let dir = std::env::temp_dir().join(format!("psms-edits-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = fixture(&dir.join("chat.db"));
        db.execute_batch(
            "INSERT INTO message VALUES(30,'E-A','Fixed typo',NULL,1,'iMessage',100,0,0,0,NULL,NULL,500,NULL);
             INSERT INTO message VALUES(31,'E-B',NULL,NULL,1,'iMessage',100,0,0,0,NULL,NULL,600,600);
             INSERT INTO message VALUES(32,'E-C','Untouched',NULL,1,'iMessage',100,0,0,0,NULL,NULL,NULL,NULL);
             INSERT INTO message VALUES(40,'E-D','Too new',NULL,1,'iMessage',100,0,0,0,NULL,NULL,700,NULL);",
        )
        .unwrap();
        let changes = read_changes(&db, 0, 32).unwrap();
        assert_eq!(
            changes,
            [
                Change {
                    guid: "E-A".into(),
                    text: "Fixed typo".into(),
                    unsent: false,
                    at: 500
                },
                Change {
                    guid: "E-B".into(),
                    text: String::new(),
                    unsent: true,
                    at: 600
                },
            ]
        );
        assert!(
            read_changes(&db, 600, 40)
                .unwrap()
                .iter()
                .all(|c| c.guid == "E-D")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_the_shortcut_copy_of_a_text() {
        let item = Item {
            rowid: 1,
            guid: "g".into(),
            text: "Hi there".into(),
            from_me: false,
            sender: "+15550142".into(),
            at: 1000,
            service: "SMS".into(),
            chat_guid: String::new(),
            chat_identifier: "+15550142".into(),
            chat_name: String::new(),
            group: false,
            members: vec![],
            files: vec![],
            reaction: None,
            reply_to: None,
        };
        let recent = vec![
            (
                "a".to_string(),
                json!({"sender": "+1 (555) 014-2", "body": "Hi there"}),
                1010,
            ),
            (
                "b".to_string(),
                json!({"sender": "+15550142", "body": "Hi there", "source": "messages"}),
                1010,
            ),
        ];
        assert_eq!(duplicate(&recent, &item), Some(("a".into(), false)));
        assert_eq!(duplicate(&recent[1..], &item), None);
        assert!(same_party("+1 555 0142", "15550142") && !same_party("+15550142", "+15550143"));
    }

    /// Copies a made-up Messages database into a running archive and reads it back, like a
    /// client would: PSMS_MESSAGES_TEST="<api url> <owner token>".
    #[test]
    #[ignore]
    fn copies_messages_into_a_running_archive() {
        let config = std::env::var("PSMS_MESSAGES_TEST").unwrap();
        let (base, token) = config.split_once(' ').unwrap();
        let inbox = Inbox::Local(Api::new(base, token));
        let key = "t".repeat(32);
        let dir = std::env::temp_dir().join(format!("psms-sync-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let chat_path = dir.join("chat.db");
        let chat = fixture(&chat_path);
        unsafe { std::env::set_var("PSMS_CHAT_DB", &chat_path) };
        let settings = Connection::open_in_memory().unwrap();
        set_mode(&settings, Mode::All).unwrap();
        // The iPhone Shortcut already sent the one-to-one text, with Contacts' formatting.
        let shortcut = uuid::Uuid::new_v4().to_string();
        let plain = seal_json(
            &key,
            &json!({"sender": "+1 (555) 014-2", "body": "Hello from Messages"}),
        )
        .unwrap();
        inbox
            .call(
                reqwest::Method::POST,
                "/messages",
                Some(json!({"id": shortcut, "sender": "papamo:encrypted:v1", "body": plain})),
            )
            .unwrap();
        let photo = dir.join("IMG_2.png");
        std::fs::write(&photo, b"\x89PNG pretend photo").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let date = (now - 60 - 978_307_200) * 1_000_000_000;
        chat.execute_batch(&format!(
            "INSERT INTO message VALUES(20,'S-A','Hello from Messages',NULL,1,'SMS',{date},0,0,0,NULL,NULL,NULL,NULL);
             INSERT INTO message VALUES(21,'S-B','Look at this',NULL,2,'iMessage',{date},0,0,0,NULL,NULL,NULL,NULL);
             INSERT INTO message VALUES(22,'S-C',NULL,NULL,0,'iMessage',{date},1,0,2000,'p:0/S-B',NULL,NULL,NULL);
             INSERT INTO message VALUES(23,'S-D','Nice!',NULL,0,'iMessage',{date},1,0,0,NULL,'S-B',NULL,NULL);
             INSERT INTO chat_message_join VALUES(1,20),(2,21),(2,22),(2,23);
             INSERT INTO attachment VALUES(5,'{}','image/png','IMG_2.png',22,0);
             INSERT INTO message_attachment_join VALUES(21,5);",
            photo.display()
        )).unwrap();
        assert_eq!(sync(&settings, &key, &inbox).unwrap(), 4);
        assert_eq!(sync(&settings, &key, &inbox).unwrap(), 0);
        let list = inbox
            .call(reqwest::Method::GET, "/messages?limit=40", None)
            .unwrap();
        let messages = list["messages"].as_array().unwrap();
        assert!(
            messages.iter().all(|m| m["id"] != json!(shortcut)),
            "the Shortcut copy is replaced"
        );
        let open = |m: &Value| open_json(&key, m["body"].as_str().unwrap()).unwrap();
        let photo_message = messages
            .iter()
            .find(|m| open(m)["body"] == "Look at this")
            .unwrap();
        let group = chat_id("iMessage;+;chat777", &key);
        assert_eq!(photo_message["chat_id"], json!(group));
        assert_eq!(photo_message["attachments"], 1);
        let reaction = open_json(
            &key,
            photo_message["reactions"][0]["envelope"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            (reaction["kind"].as_str(), reaction["from"].as_str()),
            (Some("love"), Some("me"))
        );
        let id = photo_message["id"].as_str().unwrap();
        let file = inbox
            .call(
                reqwest::Method::GET,
                &format!("/messages/{id}/attachments/0"),
                None,
            )
            .unwrap();
        let bytes = STANDARD.decode(file["envelope"].as_str().unwrap()).unwrap();
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
        let clear = Aes256Gcm::new_from_slice(key.as_bytes())
            .unwrap()
            .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
            .unwrap();
        assert_eq!(clear, b"\x89PNG pretend photo");
        let reply = messages
            .iter()
            .map(open)
            .find(|v| v["body"] == "Nice!")
            .unwrap();
        assert_eq!(
            (reply["reply_to"].as_str(), reply["direction"].as_str()),
            (Some(id), Some("out"))
        );
        let chats = inbox.call(reqwest::Method::GET, "/chats", None).unwrap();
        let titles: Vec<String> = chats["chats"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                open_json(&key, c["envelope"].as_str().unwrap()).unwrap()["title"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert!(
            titles.contains(&"Lunch crew".to_string()) && titles.contains(&"+15550142".to_string())
        );
        // Taking the tapback back removes it.
        chat.execute_batch(&format!(
            "INSERT INTO message VALUES(24,'S-E',NULL,NULL,0,'iMessage',{date},1,0,3000,'p:0/S-B',NULL,NULL,NULL);
             INSERT INTO chat_message_join VALUES(2,24);"
        )).unwrap();
        assert_eq!(sync(&settings, &key, &inbox).unwrap(), 1);
        let again = inbox
            .call(
                reqwest::Method::GET,
                &format!("/messages?chat={group}"),
                None,
            )
            .unwrap();
        assert!(
            again["messages"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m.get("reactions").is_none())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
