use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{Engine as _, engine::general_purpose::STANDARD};
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
