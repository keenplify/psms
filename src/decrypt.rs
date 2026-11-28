use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{Engine as _, engine::general_purpose::STANDARD};

/// Conversation name for the message a shortcut sends when it is run by hand.
const SETUP_SENDER: &str = "pSMS setup";
const SETUP_BODY: &str = "Setup check: this shortcut can reach your inbox. Incoming texts will appear as their own conversations.";

#[allow(dead_code)]
pub fn decrypt(key: &str, body: &str) -> Option<(String, String)> {
    open_message(key, body).map(|(sender, text, _)| (sender, text))
}

/// Like `decrypt`, and also reports replies sent from a computer, which store the other
/// person as `sender` and `"direction": "out"`.
pub fn open_message(key: &str, body: &str) -> Option<(String, String, bool)> {
    let bytes = STANDARD.decode(body).ok()?;
    if bytes.len() <= 28 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).ok()?;
    let clear = cipher
        .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&clear).ok()?;
    let sender: String = json.get("sender")?.as_str()?.into();
    let text: String = json.get("body")?.as_str()?.into();
    // Older shortcuts encrypted an empty message when run without a Message input.
    if sender.trim().is_empty() && text.trim().is_empty() {
        return Some((SETUP_SENDER.into(), SETUP_BODY.into(), false));
    }
    let outgoing = json.get("direction").and_then(|d| d.as_str()) == Some("out");
    Some((sender, text, outgoing))
}

fn open_json(key: &str, envelope: &str) -> Option<serde_json::Value> {
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
/// Shown by the CLI and MCP server when the recovery key is for another archive.
#[allow(dead_code)]
pub const WRONG_KEY: &str = "This recovery key doesn't open this archive's messages. Use the key or recovery kit made for this archive.";

/// True when `rows` hold sealed messages and the key opens none of them, so a wrong key is
/// reported instead of an empty or locked result.
#[allow(dead_code)]
pub fn opens_none<'a>(key: &str, rows: impl IntoIterator<Item = &'a serde_json::Value>) -> bool {
    let mut sealed = rows
        .into_iter()
        .filter(|r| r["sender"] == "papamo:encrypted:v1")
        .filter_map(|r| r["body"].as_str())
        .peekable();
    sealed.peek().is_some() && sealed.all(|body| open_message(key, body).is_none())
}

/// Opens a listed message for the CLI and MCP server: its sender and text, plus what pSMS for
/// Mac adds (direction, group chat, files, the message it replies to) and its tapbacks.
#[allow(dead_code)]
pub fn enrich(entry: &mut serde_json::Value, key: &str) {
    use serde_json::json;
    if entry["sender"] != "papamo:encrypted:v1" {
        return;
    }
    let Some(body) = entry["body"].as_str().map(String::from) else {
        return;
    };
    if let Some((sender, text, outgoing)) = open_message(key, &body) {
        entry["sender"] = json!(sender);
        entry["body"] = json!(text);
        if outgoing {
            entry["direction"] = json!("out");
        }
    }
    if let Some(details) = open_json(key, &body) {
        for field in ["attachments", "reply_to", "thread", "service", "source"] {
            if !details[field].is_null() {
                entry[field] = details[field].clone();
            }
        }
    }
    if let Some(reactions) = entry["reactions"].as_array().cloned() {
        let events: Vec<serde_json::Value> = reactions
            .iter()
            .filter_map(|r| open_json(key, r["envelope"].as_str()?))
            .collect();
        let change = |v: &serde_json::Value| matches!(v["kind"].as_str(), Some("edit" | "unsend"));
        entry["reactions"] = json!(
            events
                .iter()
                .filter(|v| !change(v))
                .map(|r| json!({"kind": r["kind"], "from": r["from"]}))
                .collect::<Vec<_>>()
        );
        // Edited or unsent on the sender's device after it was copied.
        if let Some(latest) = events
            .iter()
            .filter(|v| change(v))
            .max_by_key(|v| v["at"].as_i64().unwrap_or_default())
        {
            if latest["kind"] == "unsend" {
                entry["unsent"] = json!(true);
                entry["body"] = json!("");
            } else {
                entry["edited"] = json!(true);
                entry["body"] = latest["text"].clone();
            }
        }
    }
}
/// Opens a listed chat: its title, members and whether it is a group.
#[allow(dead_code)]
pub fn chat(row: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
    let details = open_json(key, row["envelope"].as_str()?)?;
    Some(serde_json::json!({
        "id": row["id"], "title": details["title"], "group": details["group"],
        "members": details["members"], "send_to": details["identifier"], "updated_at": row["updated_at"],
    }))
}
/// Opens a sealed file.
#[allow(dead_code)]
pub fn open_file(key: &str, envelope: &str) -> Option<Vec<u8>> {
    let bytes = STANDARD.decode(envelope).ok()?;
    if bytes.len() <= 28 {
        return None;
    }
    Aes256Gcm::new_from_slice(key.as_bytes())
        .ok()?
        .decrypt(Nonce::from_slice(&bytes[..12]), &bytes[12..])
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seal(key: &str, clear: &str) -> String {
        let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).unwrap();
        let nonce = [7u8; 12];
        let mut out = nonce.to_vec();
        out.extend(
            cipher
                .encrypt(Nonce::from_slice(&nonce), clear.as_bytes())
                .unwrap(),
        );
        STANDARD.encode(out)
    }
    #[test]
    fn a_key_for_another_archive_is_reported() {
        let mine = "0123456789abcdefghijklmnopqrstuv";
        let other = "vutsrqponmlkjihgfedcba9876543210";
        let row = |body: String| serde_json::json!({"sender":"papamo:encrypted:v1","body":body});
        let rows = [
            row(seal(mine, r#"{"sender":"Ana","body":"hi"}"#)),
            row(seal(mine, r#"{"sender":"Ben","body":"yo"}"#)),
        ];
        assert!(!opens_none(mine, &rows));
        assert!(opens_none(other, &rows));
        // One readable message is enough, and nothing sealed means nothing to judge by.
        let mixed = [
            rows[0].clone(),
            row(seal(other, r#"{"sender":"Cy","body":"x"}"#)),
        ];
        assert!(!opens_none(mine, &mixed));
        assert!(!opens_none(
            other,
            &[serde_json::json!({"sender":"plain","body":"x"})]
        ));
        assert!(!opens_none(other, &[] as &[serde_json::Value]));
    }
    #[test]
    fn manual_shortcut_runs_read_as_setup_checks() {
        let key = "0123456789abcdef0123456789abcdef";
        let empty = seal(key, r#"{"sender":"","body":""}"#);
        assert_eq!(empty.len(), 68);
        assert_eq!(decrypt(key, &empty).unwrap().0, SETUP_SENDER);
        let real = seal(key, r#"{"sender":"+15550100","body":"pSMS test one"}"#);
        assert_eq!(
            decrypt(key, &real).unwrap(),
            ("+15550100".into(), "pSMS test one".into())
        );
    }
}
