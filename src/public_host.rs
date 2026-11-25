use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{net::IpAddr, time::Duration};

pub fn client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(4))
        .build()
}
pub fn proof(secret: &str, challenge: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("psms-domain-v1/{secret}/{challenge}"))
    )
}
pub fn valid_host(host: &str) -> bool {
    host.len() <= 253
        && host.contains('.')
        && host.parse::<IpAddr>().is_err()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
pub async fn verify(http: &Client, host: &str, secret: &str) -> bool {
    if !valid_host(host) {
        return false;
    }
    let nonce = uuid::Uuid::new_v4();
    let Some(mut r) = http
        .get(format!("https://{host}/.well-known/psms/verify/{nonce}"))
        .header("ngrok-skip-browser-warning", "1")
        .send()
        .await
        .ok()
        .filter(|r| r.status().is_success())
    else {
        return false;
    };
    let expected = proof(secret, &nonce.to_string());
    let mut buf = Vec::new();
    loop {
        match r.chunk().await {
            Ok(Some(chunk)) if buf.len() + chunk.len() <= 64 => buf.extend_from_slice(&chunk),
            Ok(None) => return buf == expected.as_bytes(),
            _ => return false,
        }
    }
}

#[derive(Deserialize)]
struct Dns {
    #[serde(rename = "Answer", default)]
    answers: Vec<Record>,
}
#[derive(Deserialize)]
struct Record {
    #[serde(rename = "type")]
    kind: u16,
    data: String,
}

fn reverse(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v) => format!(
            "{}.in-addr.arpa",
            v.octets()
                .iter()
                .rev()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(".")
        ),
        IpAddr::V6(v) => format!(
            "{}.ip6.arpa",
            format!("{:032x}", u128::from(v))
                .chars()
                .rev()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(".")
        ),
    }
}
async fn dns(http: &Client, name: &str, kind: &str) -> Option<Vec<Record>> {
    Some(
        http.get("https://cloudflare-dns.com/dns-query")
            .header("accept", "application/dns-json")
            .query(&[("name", name), ("type", kind)])
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .json::<Dns>()
            .await
            .ok()?
            .answers,
    )
}
pub async fn detect(secret: &str) -> Option<String> {
    let http = client().ok()?;
    let trace = http
        .get("https://www.cloudflare.com/cdn-cgi/trace")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .text()
        .await
        .ok()?;
    let ip: IpAddr = trace
        .lines()
        .find_map(|s| s.strip_prefix("ip="))?
        .parse()
        .ok()?;
    for record in dns(&http, &reverse(ip), "PTR")
        .await?
        .into_iter()
        .filter(|r| r.kind == 12)
        .take(3)
    {
        let host = record.data.trim_end_matches('.').to_ascii_lowercase();
        if valid_host(&host)
            && dns(&http, &host, if ip.is_ipv4() { "A" } else { "AAAA" })
                .await
                .unwrap_or_default()
                .iter()
                .any(|r| matches!(r.kind, 1 | 28) && r.data.parse::<IpAddr>() == Ok(ip))
            && verify(&http, &host, secret).await
        {
            return Some(host);
        }
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reverse_and_validate() {
        assert_eq!(reverse("1.2.3.4".parse().unwrap()), "4.3.2.1.in-addr.arpa");
        for h in [
            "a.b/else",
            "a.b:123",
            "::1",
            "1.2.3.4",
            "bad..host",
            "a.-host",
        ] {
            assert!(!valid_host(h));
        }
        assert!(valid_host("my-pc.example.org"));
        assert_ne!(proof("a", "b"), proof("c", "b"));
    }
}
