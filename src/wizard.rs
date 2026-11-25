use crate::{cli::ServerOptions, daemon, managed_tunnel, onboard, storage};
use clap::{Args, ValueEnum};
use dialoguer::{Input, Password, Select};
use rusqlite::{Connection, OpenFlags};
use std::{
    error::Error,
    io::IsTerminal,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Copy, ValueEnum)]
pub enum Provider {
    Direct,
    Ngrok,
    Cloudflare,
}
#[derive(Args)]
pub struct Options {
    /// Existing complete HTTPS messages endpoint. Generate files without running discovery.
    #[arg(long,conflicts_with_all=["domain","provider","name","zone","credential_file"])]
    endpoint: Option<String>,
    /// Destination for the private shortcut and recovery key. Must be a new directory.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Hostname of an existing HTTPS route or custom tunnel domain.
    #[arg(long, conflicts_with = "name")]
    domain: Option<String>,
    /// Choose an existing domain (default) or a persistent tunnel.
    #[arg(long, value_enum)]
    provider: Option<Provider>,
    /// Preferred tunnel label. Defaults to this computer's DNS-safe name.
    #[arg(long)]
    name: Option<String>,
    /// DNS zone for a named Cloudflare tunnel.
    #[arg(long)]
    zone: Option<String>,
    /// ngrok authtoken file or an existing Cloudflare named-tunnel token file.
    #[arg(long)]
    credential_file: Option<PathBuf>,
    /// After writing the shortcut, wait this many seconds for a new encrypted upload.
    #[arg(long,value_parser=clap::value_parser!(u32).range(1..=86400))]
    wait: Option<u32>,
}
type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn ask(label: &str, default: Option<String>, interactive: bool) -> Result<String> {
    if !interactive {
        return default
            .ok_or_else(|| format!("{label} is required. See psms onboard --help.").into());
    }
    let mut input = Input::<String>::new().with_prompt(label);
    if let Some(value) = default {
        input = input.default(value);
    }
    Ok(input.interact_text()?)
}
async fn endpoint(path: &Path, opts: &Options) -> Result<String> {
    let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let detected = if opts.domain.is_none() && opts.provider.is_none() {
        managed_tunnel::discover(path).await?
    } else {
        None
    };
    let provider = match opts.provider {
        Some(p) => p,
        None if terminal && opts.domain.is_none() => match Select::new()
            .with_prompt("Domain setup")
            .items([
                "Existing HTTPS domain",
                "Named ngrok tunnel",
                "Named Cloudflare tunnel",
            ])
            .default(0)
            .interact()?
        {
            1 => Provider::Ngrok,
            2 => Provider::Cloudflare,
            _ => Provider::Direct,
        },
        None => Provider::Direct,
    };
    let suggested = crate::hostname_label::suggested_name(
        &hostname::get().unwrap_or_default().to_string_lossy(),
    );
    let (provider, host) = match (provider, opts.domain.as_deref()) {
        (Provider::Direct, _) => (
            "direct",
            ask(
                "Public hostname",
                opts.domain.clone().or(detected),
                terminal && opts.domain.is_none(),
            )?,
        ),
        (Provider::Ngrok, Some(host)) => ("ngrok", host.into()),
        (Provider::Cloudflare, Some(host)) => ("cloudflare", host.into()),
        (p, None) => {
            let name = ask(
                "Tunnel name",
                opts.name.clone().or_else(|| Some(suggested.clone())),
                terminal && opts.name.is_none(),
            )?;
            if name.contains('.')
                || name.len() > 63
                || !crate::public_host::valid_host(&format!("{name}.example"))
            {
                return Err("Enter one DNS label for the tunnel name.".into());
            }
            match p {
                Provider::Ngrok => (
                    if name == suggested && opts.name.is_none() {
                        "ngrok-default"
                    } else {
                        "ngrok"
                    },
                    format!("{name}.ngrok.app"),
                ),
                _ => (
                    "cloudflare",
                    format!(
                        "{name}.{}",
                        ask(
                            "Cloudflare DNS zone",
                            opts.zone.clone(),
                            terminal && opts.zone.is_none()
                        )?
                    ),
                ),
            }
        }
    };
    let credential = match (provider, opts.credential_file.as_ref()) {
        ("cloudflare", Some(f)) => f.canonicalize()?.to_string_lossy().to_string(),
        ("ngrok" | "ngrok-default", Some(f)) => std::fs::read_to_string(f)?,
        ("ngrok" | "ngrok-default", None) if terminal => Password::new()
            .with_prompt("ngrok authtoken (leave blank to reuse saved token)")
            .allow_empty_password(true)
            .interact()?,
        _ => String::new(),
    };
    managed_tunnel::configure(
        path,
        provider.into(),
        host.trim()
            .trim_start_matches("https://")
            .trim_end_matches('/')
            .to_lowercase(),
        credential,
    )
    .await
}
pub async fn run(path: &Path, server: &ServerOptions, opts: Options) -> Result<()> {
    let folder = opts.output.clone().unwrap_or_else(|| {
        PathBuf::from(format!("pSMS-pairing-{}", uuid::Uuid::new_v4().simple()))
    });
    if folder.try_exists()? {
        return Err("Choose a new pairing directory.".into());
    }
    let url = if let Some(url) = &opts.endpoint {
        url.clone()
    } else {
        daemon::init(path, server)?;
        endpoint(path, &opts).await?
    };
    let token = match std::env::var(crate::env_name!("TOKEN")) {
        Ok(v) => v,
        Err(_) => Connection::open_with_flags(
            storage::local_state(path),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?
        .query_row(
            "SELECT upload FROM local_credentials WHERE singleton=1",
            [],
            |r| r.get(0),
        )?,
    };
    let mut initial = None;
    if opts.wait.is_some() {
        initial = Some(crate::manage::newest(path, server).await?);
    }
    onboard::prepare(&url, &folder, &token)?;
    if let (Some(seconds), Some(before)) = (opts.wait, initial) {
        let wait = async {
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if let Some(latest) = crate::manage::newest(path, server).await?
                    && Some(latest) != before
                {
                    return Result::<()>::Ok(());
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(seconds.into()), wait)
            .await
            .map_err(|_| "Timed out waiting for delivery. Your pairing files were saved.")??;
        println!("Received a new message.");
    }
    Ok(())
}
