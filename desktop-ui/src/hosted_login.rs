use std::{collections::HashMap, net::TcpListener, time::Duration};

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Payload},
};
use axum::{
    Router,
    extract::Query,
    http::{HeaderMap, StatusCode},
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use ring::{agreement, rand::SystemRandom};
use sha2::{Digest, Sha256};

pub struct Attempt {
    socket: TcpListener,
    private: agreement::EphemeralPrivateKey,
    state: String,
}

pub struct Account {
    pub token: String,
    pub username: String,
    pub used: u64,
    pub limit: Option<u64>,
    /// True on Plus or Enterprise (or for admins), which can sync a self-hosted archive.
    pub plus: bool,
}

impl Attempt {
    pub fn start() -> Result<(Self, String), &'static str> {
        let socket =
            TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "Cannot open a local sign-in port.")?;
        socket
            .set_nonblocking(true)
            .map_err(|_| "Cannot start sign-in.")?;
        let port = socket
            .local_addr()
            .map_err(|_| "Cannot read local sign-in port.")?
            .port();
        let private =
            agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &SystemRandom::new())
                .map_err(|_| "Cannot create sign-in key.")?;
        let public = private
            .compute_public_key()
            .map_err(|_| "Cannot create sign-in key.")?;
        let state = uuid::Uuid::new_v4().simple().to_string();
        let url = format!(
            "https://www.papamo.dev/login#desktop={port}&state={state}&key={}",
            B64.encode(public.as_ref())
        );
        Ok((
            Self {
                socket,
                private,
                state,
            },
            url,
        ))
    }

    pub fn wait(self) -> Result<Account, &'static str> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "Could not start sign-in.")?
            .block_on(self.receive("https://na.sms.papamo.dev/free/v1/me"))
    }

    async fn receive(self, account_url: &str) -> Result<Account, &'static str> {
        let addr = self
            .socket
            .local_addr()
            .map_err(|_| "Sign-in port closed.")?
            .to_string();
        let socket =
            tokio::net::TcpListener::from_std(self.socket).map_err(|_| "Sign-in port closed.")?;
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let state = self.state.clone();
        let route = Router::new().route(
            "/papamo/return",
            get(
                move |headers: HeaderMap, Query(params): Query<HashMap<String, String>>| {
                    let valid = headers.get("host").and_then(|v| v.to_str().ok())
                        == Some(addr.as_str())
                        && params.get("state") == Some(&state);
                    let sent = valid
                        && params
                            .get("envelope")
                            .filter(|v| v.len() < 512)
                            .and_then(|v| B64.decode(v).ok())
                            .filter(|v| v.len() == 157)
                            .is_some_and(|msg| tx.try_send(msg).is_ok());
                    async move {
                        let (status, text) = if sent {
                            (
                                StatusCode::OK,
                                "Return to pSMS to continue. You can close this tab.",
                            )
                        } else {
                            (
                                StatusCode::BAD_REQUEST,
                                "Invalid sign-in request. Start again in pSMS.",
                            )
                        };
                        (
                            status,
                            [
                                ("cache-control", "no-store"),
                                ("referrer-policy", "no-referrer"),
                            ],
                            text,
                        )
                    }
                },
            ),
        );
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(socket, route)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
        });
        let msg = tokio::time::timeout(Duration::from_secs(300), rx.recv())
            .await
            .ok()
            .flatten()
            .ok_or("Sign-in expired. Try again.")?;
        let _ = stop.send(());
        let _ = tokio::time::timeout(Duration::from_secs(1), server).await;

        let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &msg[..65]);
        let key = agreement::agree_ephemeral(self.private, &peer, |s| Sha256::digest(s))
            .map_err(|_| "Invalid sign-in response.")?;
        let token = Aes256Gcm::new(&key)
            .decrypt(
                aes_gcm::Nonce::from_slice(&msg[65..77]),
                Payload {
                    msg: &msg[77..],
                    aad: self.state.as_bytes(),
                },
            )
            .map_err(|_| "Could not verify the sign-in response.")?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Cannot connect to Papamo.")?;
        let bearer = std::str::from_utf8(&token).map_err(|_| "Invalid account credential.")?;
        let response = client
            .get(account_url)
            .bearer_auth(bearer)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|_| "Could not verify your Papamo account.")?;
        let me: serde_json::Value = response
            .json()
            .await
            .map_err(|_| "Invalid account response.")?;
        Ok(Account {
            username: me["username"]
                .as_str()
                .ok_or("Missing account name.")?
                .into(),
            used: me["used"].as_u64().ok_or("Missing archive usage.")?,
            limit: me["limit"].as_u64(),
            plus: me["admin"] == true || me["plan"]["id"].as_str().is_some_and(|p| p != "free"),
            token: bearer.into(),
        })
    }
}
impl Account {
    pub fn sign_out(self) {
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            rt.block_on(async {
                if let Ok(client) = reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                {
                    let _ = client
                        .delete("https://na.sms.papamo.dev/free/v1/auth/session")
                        .bearer_auth(self.token)
                        .send()
                        .await;
                }
            });
        }
    }
}
fn runtime() -> Result<tokio::runtime::Runtime, &'static str> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not start the account connection.")
}

fn client() -> Result<reqwest::Client, &'static str> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Cannot connect to Papamo.")
}

/// Whether this papamo.dev account already has a phone sending to it.
pub fn has_phone(token: &str) -> Result<bool, &'static str> {
    runtime()?.block_on(async {
        let list: serde_json::Value = client()?
            .get("https://na.sms.papamo.dev/free/v1/phones")
            .bearer_auth(token)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|_| "Could not check your phones. Try signing in again.")?
            .json()
            .await
            .map_err(|_| "Invalid phone list.")?;
        Ok(list["phones"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|p| p["kind"] != "computer"))
    })
}

#[allow(dead_code)]
pub fn pair(token: &str, folder: &std::path::Path, username: &str) -> Result<(), &'static str> {
    runtime()?.block_on(async {
        let payload: serde_json::Value = client()?
            .post("https://na.sms.papamo.dev/free/v1/pair")
            .bearer_auth(token)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|_| "Could not pair this phone. Try signing in again.")?
            .json()
            .await
            .map_err(|_| "Invalid pairing response.")?;
        let upload = payload["token"]
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("Invalid upload credential.")?;
        crate::onboard::prepare(
            "https://na.sms.papamo.dev/free/v1/messages",
            folder,
            upload,
            Some(username),
        )
        .map_err(
            |_| "Could not save the private shortcut. Check your archive folder permissions.",
        )?;
        std::fs::write(folder.join("papamo-hosted"), "1")
            .map_err(|_| "Could not save the pairing details.")
    })
}

pub fn verify(token: &str, folder: &std::path::Path) -> Result<bool, &'static str> {
    let key = std::fs::read_to_string(folder.join("archive-recovery-key.txt"))
        .map_err(|_| "The pairing recovery key could not be opened.")?;
    runtime()?.block_on(async {
        let page: serde_json::Value = client()?
            .get("https://na.sms.papamo.dev/free/v1/messages?limit=50")
            .bearer_auth(token)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|_| "Could not read your hosted archive. Try signing in again.")?
            .json()
            .await
            .map_err(|_| "Invalid archive response.")?;
        Ok(page["messages"]
            .as_array()
            .ok_or("Invalid message list.")?
            .iter()
            .any(|m| {
                m["body"]
                    .as_str()
                    .and_then(|b| crate::archive::decrypt(key.trim(), b))
                    .is_some()
            }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_ignores_wrong_state_and_verifies_the_hosted_session() {
        runtime().unwrap().block_on(async {
            let (attempt, login) = Attempt::start().unwrap();
            let url = url::Url::parse(&login).unwrap();
            let launch: HashMap<_, _> =
                url::form_urlencoded::parse(url.fragment().unwrap().as_bytes()).collect();
            let state = launch["state"].to_string();
            let peer = B64.decode(launch["key"].as_bytes()).unwrap();
            let callback = format!("http://127.0.0.1:{}/papamo/return", launch["desktop"]);
            let browser = agreement::EphemeralPrivateKey::generate(
                &agreement::ECDH_P256,
                &SystemRandom::new(),
            )
            .unwrap();
            let mut envelope = browser.compute_public_key().unwrap().as_ref().to_vec();
            let key = agreement::agree_ephemeral(
                browser,
                &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, peer),
                |s| Sha256::digest(s),
            )
            .unwrap();
            let token = "d".repeat(64);
            let iv = [3; 12];
            envelope.extend(iv);
            envelope.extend(
                Aes256Gcm::new(&key)
                    .encrypt(
                        (&iv).into(),
                        Payload {
                            msg: token.as_bytes(),
                            aad: state.as_bytes(),
                        },
                    )
                    .unwrap(),
            );
            let encoded = B64.encode(envelope);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let me = format!("http://{}/me", listener.local_addr().unwrap());
            tokio::spawn(async move {
                axum::serve(
                    listener,
                    Router::new().route(
                        "/me",
                        get(move |headers: HeaderMap| async move {
                            assert_eq!(headers["authorization"], format!("Bearer {token}"));
                            axum::Json(
                                serde_json::json!({"username":"tester","used":17,"limit":100}),
                            )
                        }),
                    ),
                )
                .await
                .unwrap();
            });
            let returned = tokio::spawn(async move { attempt.receive(&me).await });
            let http = client().unwrap();
            for (nonce, status) in [("wrong", 400), (state.as_str(), 200)] {
                let res = http
                    .get(&callback)
                    .query(&[("state", nonce), ("envelope", &encoded)])
                    .send()
                    .await
                    .unwrap();
                assert_eq!(res.status().as_u16(), status);
            }
            let account = returned.await.unwrap().unwrap();
            assert_eq!(account.username, "tester");
            assert_eq!(account.used, 17);
            assert_eq!(account.limit, Some(100));
        });
    }
}
