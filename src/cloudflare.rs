use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::{Child, Command};
use uuid::Uuid;
fn connector() -> Command {
    let mut cmd = Command::new("cloudflared");
    cmd.kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(|k| {
            k.starts_with("PSMS_") || k.starts_with("PAPAMO_") || k == "TUNNEL_TOKEN"
        }) {
            cmd.env_remove(key);
        }
    }
    cmd
}
async fn run(cmd: &mut Command, reason: &'static str) -> Result<(), &'static str> {
    let status = tokio::time::timeout(Duration::from_secs(16), cmd.status())
        .await
        .map_err(|_| "Cloudflare timed out. Try again.")?
        .map_err(|_| "Install cloudflared and make it available on PATH.")?;
    if status.success() {
        Ok(())
    } else {
        Err(reason)
    }
}
pub async fn start(
    db: &Path,
    host: &str,
    origin: std::net::SocketAddr,
    token: &str,
) -> Result<Child, &'static str> {
    let mut cmd = connector();
    if !token.is_empty() {
        let file =
            std::fs::canonicalize(token).map_err(|_| "Choose a readable Cloudflare token file.")?;
        cmd.args(["tunnel", "run", "--token-file"]).arg(file);
    } else {
        let mut filename = db.as_os_str().to_owned();
        filename.push(".cloudflared.json");
        let credentials = std::path::PathBuf::from(filename);
        if !credentials.is_file() {
            run(
                connector()
                    .args(["tunnel", "create", "--credentials-file"])
                    .arg(&credentials)
                    .arg(format!("psms-{}", Uuid::new_v4().simple())),
                "Could not create the named tunnel. Sign in to Cloudflare for your DNS zone first.",
            )
            .await?;
        }
        let identity: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&credentials).map_err(|_| "Missing Cloudflare credentials.")?,
        )
        .map_err(|_| "Invalid Cloudflare credentials.")?;
        let id = identity["TunnelID"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or("Invalid tunnel identity.")?
            .to_string();
        run(connector().args(["tunnel","route","dns",&id,host]),"Could not register this name in your Cloudflare zone. Check permissions or choose an available name.").await?;
        let mut config = db.as_os_str().to_owned();
        config.push(".cloudflared.yml");
        let config = std::path::PathBuf::from(config);
        let contents = serde_json::json!({"tunnel":id,"credentials-file":credentials.canonicalize().map_err(|_|"Could not locate tunnel credentials.")?,"ingress":[{"hostname":host,"service":format!("http://{origin}")},{"service":"http_status:404"}]});
        let mut tmp = tempfile::NamedTempFile::new_in(
            config
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .map_err(|_| "Could not stage connector configuration.")?;
        serde_json::to_writer(&mut tmp, &contents)
            .map_err(|_| "Could not write connector configuration.")?;
        tmp.persist(&config)
            .map_err(|_| "Could not save connector configuration.")?;
        cmd.args(["tunnel", "--config"])
            .arg(config)
            .args(["run", &id]);
    }
    cmd.spawn()
        .map_err(|_| "Could not start cloudflared. Check the installation and PATH.")
}
