//! Contact cards in the local archive, sealed with the archive key like messages.
//! The phone's copy comes from its address book; edits made here are stored as the manual copy
//! and win when both exist.
use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui_kit::{Image, ImageFormat};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

/// A phone number or email with its label, such as "mobile", "work" or any custom tag.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Labeled {
    pub label: String,
    pub value: String,
}
#[derive(Clone, Default)]
pub struct Card {
    pub name: String,
    pub org: String,
    pub title: String,
    pub phones: Vec<Labeled>,
    pub emails: Vec<Labeled>,
    pub note: String,
    pub photo: Option<(String, String)>,
}
pub struct Entry {
    pub card: Card,
    pub edited: bool,
    pub from_phone: bool,
    pub image: Option<Arc<Image>>,
}

fn open(key: &str, envelope: &str) -> Option<Value> {
    let bytes = STANDARD.decode(envelope).ok()?;
    if bytes.len() <= 28 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).ok()?;
    let clear = cipher
        .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
        .ok()?;
    serde_json::from_slice(&clear).ok()
}

fn seal(key: &str, value: &Value) -> Option<String> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).ok()?;
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).ok()?;
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            serde_json::to_vec(value).ok()?.as_slice(),
        )
        .ok()?;
    Some(STANDARD.encode([nonce.as_slice(), &sealed].concat()))
}

fn text(v: &Value, field: &str) -> String {
    v.get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}
/// Reads `[{"label","value"}]`, also accepting the earlier plain-string lists.
fn list(v: &Value, field: &str, fallback: &str) -> Vec<Labeled> {
    v.get(field)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|item| match item {
                    Value::String(value) => Some(Labeled {
                        label: fallback.into(),
                        value: value.trim().into(),
                    }),
                    Value::Object(_) => Some(Labeled {
                        label: Some(text(item, "label"))
                            .filter(|l| !l.is_empty())
                            .unwrap_or_else(|| fallback.into()),
                        value: text(item, "value"),
                    }),
                    _ => None,
                })
                .filter(|l| !l.value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}
fn labeled(items: &[Labeled]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|l| json!({"label": l.label, "value": l.value}))
            .collect(),
    )
}
/// Reads a card in the shared vCard-style format, tolerating missing fields.
fn parse(v: &Value) -> Card {
    let photo = v.get("photo").and_then(|p| {
        let kind = p.get("type")?.as_str()?;
        let data = p.get("data")?.as_str()?;
        matches!(kind, "image/jpeg" | "image/png").then(|| (kind.to_string(), data.to_string()))
    });
    Card {
        name: text(v, "fn"),
        org: text(v, "org"),
        title: text(v, "title"),
        phones: list(v, "tel", "mobile"),
        emails: list(v, "email", "email"),
        note: text(v, "note"),
        photo,
    }
}
fn image(card: &Card) -> Option<Arc<Image>> {
    let (kind, data) = card.photo.as_ref()?;
    let format = if kind == "image/png" {
        ImageFormat::Png
    } else {
        ImageFormat::Jpeg
    };
    Some(Arc::new(Image::from_bytes(
        format,
        STANDARD.decode(data).ok()?,
    )))
}

/// Decrypts every card this key can open, keyed by contact ID.
pub fn load(db: &Connection, key: &str) -> HashMap<String, Entry> {
    let mut out = HashMap::new();
    let Ok(mut stmt) = db.prepare("SELECT contact_id, phone_card, manual_card FROM contact_cards")
    else {
        return out;
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    }) else {
        return out;
    };
    for (id, phone, manual) in rows.flatten() {
        let phone = phone.and_then(|e| open(key, &e));
        let manual = manual.and_then(|e| open(key, &e));
        let Some(value) = manual.as_ref().or(phone.as_ref()) else {
            continue;
        };
        let card = parse(value);
        out.insert(
            id,
            Entry {
                image: image(&card),
                edited: manual.is_some(),
                from_phone: phone.is_some(),
                card,
            },
        );
    }
    out
}

/// Saves edits as the manual copy, or clears them so the phone's copy shows again.
pub fn save(
    db: &Connection,
    key: &str,
    contact: &str,
    card: Option<&Card>,
) -> Result<(), &'static str> {
    let envelope = match card {
        Some(c) => {
            let mut value = json!({
                "version": 1, "fn": c.name, "org": c.org, "title": c.title,
                "tel": labeled(&c.phones), "email": labeled(&c.emails), "note": c.note,
            });
            if let Some((kind, data)) = &c.photo {
                value["photo"] = json!({"type": kind, "data": data});
            }
            Some(seal(key, &value).ok_or("Could not seal this contact.")?)
        }
        None => None,
    };
    let exists = db
        .query_row(
            "SELECT 1 FROM contact_cards WHERE contact_id=?1",
            [contact],
            |_| Ok(()),
        )
        .optional()
        .map_err(|_| "Could not read contacts.")?
        .is_some();
    let result = if exists {
        db.execute(
            "UPDATE contact_cards SET manual_card=?2, manual_updated_at=unixepoch() WHERE contact_id=?1",
            params![contact, envelope],
        )
    } else if envelope.is_some() {
        db.execute(
            "INSERT INTO contact_cards(contact_id,manual_card,manual_updated_at) VALUES(?1,?2,unixepoch())",
            params![contact, envelope],
        )
    } else {
        Ok(0)
    };
    result
        .map(|_| ())
        .map_err(|_| "Could not save this contact.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_round_trip_and_win_over_the_phone_copy() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE contact_cards(contact_id TEXT PRIMARY KEY, phone_card TEXT, phone_updated_at INTEGER, manual_card TEXT, manual_updated_at INTEGER)").unwrap();
        let key = "0123456789abcdef0123456789abcdef";
        let id = "a".repeat(64);
        let phone = seal(key, &json!({"fn": "Ana Reyes", "tel": ["+15550123"]})).unwrap();
        db.execute(
            "INSERT INTO contact_cards(contact_id,phone_card) VALUES(?1,?2)",
            params![id, phone],
        )
        .unwrap();
        let loaded = load(&db, key);
        assert_eq!(loaded[&id].card.name, "Ana Reyes");
        assert!(loaded[&id].from_phone && !loaded[&id].edited);
        let mut edited = loaded[&id].card.clone();
        edited.name = "Ana (work)".into();
        save(&db, key, &id, Some(&edited)).unwrap();
        let loaded = load(&db, key);
        assert_eq!(loaded[&id].card.name, "Ana (work)");
        assert_eq!(
            loaded[&id].card.phones,
            vec![Labeled {
                label: "mobile".into(),
                value: "+15550123".into()
            }]
        );
        edited.phones.push(Labeled {
            label: "Kylla's office".into(),
            value: "+63 992 037 1891".into(),
        });
        save(&db, key, &id, Some(&edited)).unwrap();
        assert_eq!(load(&db, key)[&id].card.phones[1].label, "Kylla's office");
        save(&db, key, &id, None).unwrap();
        assert_eq!(load(&db, key)[&id].card.name, "Ana Reyes");
        assert!(load(&db, "f".repeat(32).as_str()).is_empty());
    }
}
