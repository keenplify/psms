//! Fills contact cards from this Mac's Contacts. Each person who has texted you is matched by
//! phone number (the last ten digits, so +63 and 0 prefixes agree), and their name, company,
//! title, numbers, emails and photo are sealed with the archive key and saved as the device's
//! copy, which never overrides your own edits. Like BlueBubbles reading the Mac's address book,
//! this is on unless turned off. Each card is sent again once a day, so the Mac's copy wins back
//! over a phone that had no name for the number.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS desktop_mac_contacts(id INTEGER PRIMARY KEY CHECK(id=1), enabled INTEGER NOT NULL, matched INTEGER NOT NULL DEFAULT 0, synced_at INTEGER);
  CREATE TABLE IF NOT EXISTS desktop_mac_contact_sent(contact_id TEXT PRIMARY KEY, digest TEXT NOT NULL)";

pub fn enabled(db: &Connection) -> bool {
    db.query_row(
        "SELECT enabled FROM desktop_mac_contacts WHERE id=1",
        [],
        |r| r.get(0),
    )
    .unwrap_or(cfg!(target_os = "macos"))
}
pub fn set_enabled(db: &Connection, on: bool) -> rusqlite::Result<()> {
    db.execute_batch(SCHEMA)?;
    db.execute(
        "INSERT INTO desktop_mac_contacts(id,enabled) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled",
        [on],
    )?;
    if !on {
        db.execute("DELETE FROM desktop_mac_contact_sent", [])?;
    }
    Ok(())
}
/// How many people the last sync matched, and when.
pub fn status(db: &Connection) -> Option<(i64, Option<i64>)> {
    db.query_row(
        "SELECT matched, synced_at FROM desktop_mac_contacts WHERE id=1 AND enabled",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .ok()
}

/// Everyone in this Mac's Contacts. macOS asks for permission the first time.
#[cfg(target_os = "macos")]
pub fn read() -> Result<Vec<Value>, String> {
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", include_str!("contacts.js")])
        .output()
        .map_err(|_| "Could not open Contacts.".to_string())?;
    if !out.status.success() {
        return Err(
            "Allow pSMS to use Contacts in System Settings → Privacy & Security → Contacts.".into(),
        );
    }
    serde_json::from_slice(&out.stdout).map_err(|_| "Could not read Contacts.".into())
}
#[cfg(not(target_os = "macos"))]
pub fn read() -> Result<Vec<Value>, String> {
    Err("Reading Contacts needs pSMS for Mac.".into())
}

/// The last ten digits, which match a number written with or without a country code.
pub fn number_key(number: &str) -> Option<String> {
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    (digits.len() >= 7).then(|| digits[digits.len().saturating_sub(10)..].to_string())
}

fn photo(data: &str) -> Option<Value> {
    let bytes = STANDARD.decode(data).ok()?;
    let image = image::load_from_memory(&bytes).ok()?;
    let square = image.resize_to_fill(256, 256, image::imageops::FilterType::Lanczos3);
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode_image(&square.to_rgb8())
        .ok()?;
    Some(json!({"type": "image/jpeg", "data": STANDARD.encode(jpeg)}))
}
/// A pSMS contact card from one person in Contacts.
pub fn card(person: &Value) -> Value {
    let text = |k: &str| person[k].as_str().unwrap_or_default().trim().to_string();
    let name = [text("given"), text("family")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let labeled = |k: &str, fallback: &str| {
        person[k]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let value = e["value"].as_str()?.trim();
                let label = e["label"].as_str().unwrap_or_default().trim();
                (!value.is_empty()).then(|| {
                    json!({"label": if label.is_empty() { fallback } else { label }, "value": value})
                })
            })
            .collect::<Vec<_>>()
    };
    let mut card = json!({
        "version": 1,
        "fn": if name.is_empty() { text("org") } else { name },
        "org": text("org"), "title": text("title"),
        "tel": labeled("tel", "mobile"), "email": labeled("email", "email"),
    });
    if let Some(p) = person["photo"].as_str().and_then(photo) {
        card["photo"] = p;
    }
    card
}

/// Matches senders to Contacts and uploads changed cards. Returns how many senders matched.
/// `upload` saves one sealed card as the device copy for a contact ID.
pub fn sync(
    db: &Connection,
    key: &str,
    senders: &[String],
    upload: impl Fn(&str, String) -> Result<(), String>,
) -> Result<usize, String> {
    db.execute_batch(SCHEMA)
        .map_err(|_| "Could not save settings.")?;
    let people = read()?;
    let mut by_number = std::collections::HashMap::new();
    for person in &people {
        for tel in person["tel"].as_array().into_iter().flatten() {
            if let Some(k) = tel["value"].as_str().and_then(number_key) {
                by_number.entry(k).or_insert(person);
            }
        }
    }
    let mut matched = 0;
    for sender in senders {
        let Some(person) = number_key(sender).and_then(|k| by_number.get(&k)) else {
            continue;
        };
        matched += 1;
        let card = card(person);
        let id = crate::replies::contact_id(sender, key);
        // The day is part of the digest, so an unchanged card is still sent once a day.
        let day = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() / 86_400)
            .unwrap_or_default();
        let digest = format!("{:x}", Sha256::digest(format!("{day}\n{card}")));
        let sent: Option<String> = db
            .query_row(
                "SELECT digest FROM desktop_mac_contact_sent WHERE contact_id=?1",
                [&id],
                |r| r.get(0),
            )
            .ok();
        if sent.as_deref() == Some(digest.as_str()) {
            continue;
        }
        let Some(envelope) = crate::cards::seal(key, &card) else {
            continue;
        };
        upload(&id, envelope)?;
        let _ = db.execute(
            "INSERT INTO desktop_mac_contact_sent(contact_id,digest) VALUES(?1,?2) ON CONFLICT(contact_id) DO UPDATE SET digest=excluded.digest",
            rusqlite::params![id, digest],
        );
    }
    let _ = db.execute(
        "INSERT INTO desktop_mac_contacts(id,enabled,matched,synced_at) VALUES(1,1,?1,unixepoch())
         ON CONFLICT(id) DO UPDATE SET matched=excluded.matched, synced_at=excluded.synced_at",
        [matched as i64],
    );
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_match_with_or_without_country_codes_and_cards_are_built() {
        assert_eq!(number_key("+63 960 565 9389"), number_key("09605659389"));
        assert_eq!(number_key("(555) 010-0123"), number_key("+1 555 010 0123"));
        assert_eq!(number_key("GLOBE"), None);
        let c = card(
            &json!({"given":"Ana","family":"Reyes","org":"Papamo","title":"",
            "tel":[{"label":"","value":"+63 960 565 9389"}],"email":[{"label":"work","value":"ana@example.com"}],"photo":null}),
        );
        assert_eq!(c["fn"], "Ana Reyes");
        assert_eq!(
            c["tel"][0],
            json!({"label":"mobile","value":"+63 960 565 9389"})
        );
        assert_eq!(c["email"][0]["label"], "work");
        assert!(c.get("photo").is_none());
    }
}
