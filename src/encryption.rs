use axum::http::Uri;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use std::{env, error::Error};

pub const ACTIONS: &str = "papamo:encrypted:v1";
const OPENPGP: &str = "papamo:openpgp:v1";

#[derive(Clone, Serialize)]
pub struct Provider {
    pub marker: &'static str,
    pub name: &'static str,
    pub shortcut_url: Option<String>,
}

pub fn providers() -> Result<Vec<Provider>, Box<dyn Error>> {
    [
        (ACTIONS, "Actions", crate::env_name!("ACTIONS_SHORTCUT_URL")),
        (
            OPENPGP,
            "OpenPGP (iPGMail)",
            crate::env_name!("OPENPGP_SHORTCUT_URL"),
        ),
    ]
    .into_iter()
    .map(|(marker, name, var)| {
        let shortcut_url = env::var(var).ok();
        if let Some(url) = &shortcut_url
            && !url
                .parse::<Uri>()
                .is_ok_and(|u| u.scheme_str() == Some("https") && u.host().is_some())
        {
            return Err(format!("{var} must contain an absolute HTTPS URL").into());
        }
        Ok(Provider {
            marker,
            name,
            shortcut_url,
        })
    })
    .collect()
}

pub fn accepts(marker: &str, body: &str) -> bool {
    match marker {
        ACTIONS => STANDARD.decode(body).is_ok_and(|v| v.len() > 28),
        OPENPGP => pgp_envelope(body),
        _ => false,
    }
}

// Validate the transport envelope. Only the recipient can authenticate the plaintext.
fn pgp_envelope(body: &str) -> bool {
    let normalized = body.replace("\r\n", "\n");
    let Some(armor) = normalized
        .trim_end()
        .strip_prefix("-----BEGIN PGP MESSAGE-----\n")
        .and_then(|s| s.strip_suffix("\n-----END PGP MESSAGE-----"))
    else {
        return false;
    };
    let mut lines = armor.lines();
    loop {
        match lines.next() {
            Some("") => break,
            Some(header) if header.contains(": ") => continue,
            _ => return false,
        }
    }
    let mut encoded = String::new();
    while let Some(line) = lines.next() {
        if let Some(checksum) = line.strip_prefix('=') {
            if !STANDARD.decode(checksum).is_ok_and(|v| v.len() == 3) || lines.next().is_some() {
                return false;
            }
            break;
        }
        encoded.push_str(line);
    }
    STANDARD.decode(encoded).is_ok_and(|bytes| {
        bytes.first().is_some_and(|b| {
            let tag = if b & 0xc0 == 0xc0 {
                b & 0x3f
            } else if b & 0xc0 == 0x80 {
                (b >> 2) & 15
            } else {
                0
            };
            matches!(tag, 1 | 3) && bytes.len() > 16
        })
    })
}
