//! Replies and queued texts, sent from this computer through the Messages app. The desktop talks
//! to the local archive over the same HTTP API as the CLI and MCP server: it saves each sent
//! reply sealed with the archive key, and delivers messages other apps queued in the outbox.
use crate::api::Api;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Sends through Messages. The recipient and text are passed as script arguments, never
/// spliced into the script. A person goes through the SMS account (Text Message Forwarding)
/// when there is one, otherwise iMessage; a group goes to its Messages chat, which BlueBubbles
/// does the same way without its private helper.
pub fn send(to: &str, text: &str) -> Result<(), String> {
    run(to, text, "text")
}
/// Sends a file, such as a photo, to a person or a group chat.
pub fn send_file(to: &str, path: &std::path::Path) -> Result<(), String> {
    run(to, &path.to_string_lossy(), "file")
}
/// Whether a recipient is a Messages chat (a group) rather than a phone number or email.
pub fn is_chat(to: &str) -> bool {
    to.contains(";+;") || to.contains(";-;")
}
#[cfg(target_os = "macos")]
fn run(to: &str, payload: &str, kind: &str) -> Result<(), String> {
    const SCRIPT: &str = r#"on run argv
    set target to item 1 of argv
    set payload to item 2 of argv
    set kind to item 3 of argv
    if kind is "file" then set payload to POSIX file payload
    tell application "Messages"
        if target contains ";+;" or target contains ";-;" then
            set destination to chat id target
        else
            try
                set chosen to 1st account whose service type = SMS and enabled is true
            on error
                set chosen to 1st account whose service type = iMessage and enabled is true
            end try
            set destination to participant target of chosen
        end if
        send payload to destination
    end tell
end run"#;
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", SCRIPT, "--", to, payload, kind])
        .output()
        .map_err(|_| "Could not start Messages.".to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let error = String::from_utf8_lossy(&out.stderr);
    Err(
        if error.contains("-1743") || error.contains("Not authorized") {
            "Allow pSMS to control Messages in System Settings → Privacy & Security → Automation."
                .into()
        } else if error.contains("-1728")
            || error.contains("Can’t get account")
            || error.contains("Can’t get chat")
        {
            if is_chat(to) {
                "Messages could not find this group chat on this Mac.".into()
            } else {
                "Sign in to Messages, and turn on Text Message Forwarding on your iPhone for SMS."
                    .into()
            }
        } else {
            "Messages could not send this.".into()
        },
    )
}
#[cfg(not(target_os = "macos"))]
fn run(_: &str, _: &str, _: &str) -> Result<(), String> {
    Err("Replies need pSMS for Mac, which sends through the Messages app.".into())
}
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// The archive's private ID for a sender, as the shortcut computes it.
pub fn contact_id(sender: &str, key: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("psms-contact-v1\n{sender}\n{key}"))
    )
}

fn seal(key: &str, to: &str, text: &str) -> Option<String> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).ok()?;
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).ok()?;
    let clear = json!({"sender": to, "body": text, "direction": "out"}).to_string();
    let sealed = cipher
        .encrypt(Nonce::from_slice(&nonce), clear.as_bytes())
        .ok()?;
    Some(STANDARD.encode([nonce.as_slice(), &sealed].concat()))
}

/// The sealed request that keeps a sent reply in the archive, linked to the contact.
pub fn sent_copy(key: &str, to: &str, text: &str) -> Option<(String, Value)> {
    let id = uuid::Uuid::new_v4().to_string();
    let body = json!({
        "id": id, "sender": "papamo:encrypted:v1", "body": seal(key, to, text)?,
        "contact_id": contact_id(to, key),
    });
    Some((id, body))
}

/// Keeps a sent reply in the archive, sealed and already read.
pub fn store(api: &Api, key: &str, to: &str, text: &str) -> Result<(), &'static str> {
    let (id, body) = sent_copy(key, to, text).ok_or("Could not seal the reply.")?;
    let failed = "Sent, but could not save the reply in this archive.";
    api.call(reqwest::Method::POST, "/messages", Some(body))
        .map_err(|_| failed)?;
    api.call(
        reqwest::Method::POST,
        "/messages/read",
        Some(json!({"ids": [id]})),
    )
    .map_err(|_| failed)?;
    Ok(())
}
/// Sends what other apps queued, and reports each result. Returns how many were sent.
/// Keeps a sealed copy when the archive key is unlocked here.
pub fn deliver(api: &Api, key: Option<&str>) -> usize {
    let Ok(batch) = api.call(reqwest::Method::POST, "/outbox/claim", None) else {
        return 0;
    };
    let mut sent = 0;
    for item in batch["messages"].as_array().into_iter().flatten() {
        let (Some(id), Some(to), Some(text)) = (
            item["id"].as_str(),
            item["to"].as_str(),
            item["body"].as_str(),
        ) else {
            continue;
        };
        let outcome = match send(to, text) {
            Ok(()) => {
                sent += 1;
                if let Some(key) = key.filter(|_| !is_chat(to)) {
                    let _ = store(api, key, to, text);
                }
                json!({"status": "sent"})
            }
            Err(e) => json!({"status": "failed", "error": e}),
        };
        let _ = api.call(
            reqwest::Method::POST,
            &format!("/outbox/{id}/result"),
            Some(outcome),
        );
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sent_copies_open_as_outgoing_in_the_same_conversation() {
        let key = "0123456789abcdef0123456789abcdef";
        let (id, body) = sent_copy(key, "+15550123", "On my way").unwrap();
        assert_eq!(body["id"], id.as_str());
        assert_eq!(body["contact_id"], contact_id("+15550123", key).as_str());
        let opened = crate::archive::open_message(key, body["body"].as_str().unwrap()).unwrap();
        assert_eq!(opened, ("+15550123".into(), "On my way".into(), true));
    }
}
