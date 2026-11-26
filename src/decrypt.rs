use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{Engine as _, engine::general_purpose::STANDARD};

/// Conversation name for the message a shortcut sends when it is run by hand.
const SETUP_SENDER: &str = "pSMS setup";
const SETUP_BODY: &str = "Setup check: this shortcut can reach your inbox. Incoming texts will appear as their own conversations.";

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
    let sender: String = json.get("sender")?.as_str()?.into();
    let text: String = json.get("body")?.as_str()?.into();
    // Older shortcuts encrypted an empty message when run without a Message input.
    if sender.trim().is_empty() && text.trim().is_empty() {
        return Some((SETUP_SENDER.into(), SETUP_BODY.into()));
    }
    Some((sender, text))
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
