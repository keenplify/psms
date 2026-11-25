use crate::cli::Tunnel;
use std::{error::Error, net::SocketAddr, process::Command};

pub fn run(provider: Tunnel, bind: SocketAddr) -> Result<(), Box<dyn Error>> {
    let mut cmd = Command::new("cloudflared");
    match provider {
        Tunnel::Quick => {
            if !bind.ip().is_loopback() || bind.port() == 0 {
                return Err("quick tunnel requires a fixed loopback address".into());
            }
            cmd.args(["tunnel", "--url"]).arg(format!("http://{bind}"));
        }
        Tunnel::Cloudflare { token_file } => {
            let path = token_file.canonicalize()?;
            if !path.is_file() {
                return Err("token file must be a regular file".into());
            }
            cmd.args(["tunnel", "run", "--token-file"]).arg(path);
        }
    }
    for key in [
        "PAPAMO_TOKEN",
        "PAPAMO_OWNER_TOKEN",
        "PAPAMO_ARCHIVE_KEY",
        "TUNNEL_TOKEN",
    ] {
        cmd.env_remove(key);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(cmd.exec().into())
    }
    #[cfg(not(unix))]
    {
        if cmd.status()?.success() {
            Ok(())
        } else {
            Err("Cloudflare connector exited unsuccessfully".into())
        }
    }
}
