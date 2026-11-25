use clap::{Args, Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(
    name = "psms",
    version,
    about = "pSMS (Papamo SMS) — a private archive for encrypted phone messages",
    after_help = "Quick start: psms init\nManage the background service: psms status | psms stop\nRemote access: psms tunnel --help"
)]
pub struct Cli {
    #[command(flatten)]
    pub options: ServerOptions,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Verify and activate the latest stable release for this OS user.
    Update(crate::updater::Options),
    /// Inspect or migrate authoritative archive storage.
    Storage {
        #[command(subcommand)]
        command: crate::storage::Command,
    },
    /// Serve read-only Model Context Protocol tools over stdin/stdout.
    Mcp {
        /// Archive API base URL. Defaults to the running local archive.
        #[arg(long)]
        server: Option<String>,
        /// Owner API bearer token file (required for remote servers).
        #[arg(long)]
        token_file: Option<PathBuf>,
        /// Enable local sender and body searches with this private Actions recovery key.
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Pair archives, enable scheduled synchronization or inspect the queue.
    Sync {
        #[command(subcommand)]
        command: crate::sync::Command,
    },
    /// Read a sender ID or phone number from stdin and calculate its lookup ID offline.
    ContactId {
        /// The recovery key file generated for the originating shortcut.
        #[arg(long)]
        key_file: PathBuf,
    },
    /// Initialize the archive and start the background service (default).
    Init,
    /// Prepare a private, preconfigured Actions shortcut for this archive.
    Onboard(crate::wizard::Options),
    /// Manage scoped tokens, webhooks, sync timing, appearance and inbox messages.
    Manage {
        #[command(subcommand)]
        action: crate::manage::Action,
    },
    /// Show whether the local background service is running.
    Status,
    /// Stop the local background service.
    Stop,
    /// Show a locally generated credential. Treat the output as a secret.
    Token {
        #[arg(value_enum)]
        role: Role,
    },
    #[command(hide = true)]
    DaemonWorker,
    /// Start the HTTP service without opening a browser.
    Serve,
    /// Open the native GPUI Kit dashboard.
    Desktop,
    /// Open the native settings window.
    Settings,
    /// Expose a running local service with an optional tunnel connector.
    Tunnel {
        #[command(subcommand)]
        provider: Tunnel,
    },
}

#[derive(Subcommand)]
pub enum Tunnel {
    /// Stop the managed connector and clear its saved configuration.
    Disconnect,
    /// Show the managed connector and its latest verified endpoint.
    Status,
    /// Detect a public hostname that reaches this archive over HTTPS.
    Detect,
    /// Configure the background connector. Read its credential from stdin.
    Connect {
        provider: String,
        #[arg(default_value = "")]
        host: String,
    },
    /// Start a temporary Cloudflare HTTPS URL using cloudflared on PATH.
    Quick,
    /// Connect a remotely managed Cloudflare tunnel with a stable hostname.
    Cloudflare {
        /// File containing the tunnel connector token.
        #[arg(long, env = crate::env_name!("TUNNEL_TOKEN_FILE"), hide_env_values = true)]
        token_file: PathBuf,
    },
}

#[derive(Args)]
pub struct ServerOptions {
    /// HTTP listen address. Defaults to loopback port 47832 with automatic fallback.
    #[arg(long, global = true, env = crate::env_name!("BIND"))]
    pub bind: Option<SocketAddr>,
    /// SQLite archive path. Defaults to the user's application data directory.
    #[arg(long, global = true, env = crate::env_name!("DB_PATH"))]
    pub database: Option<PathBuf>,
    /// URL path prefix for the JSON API.
    #[arg(long, global = true, env = crate::env_name!("API_PREFIX"), default_value = "/v1")]
    pub api_prefix: String,
}

#[derive(Copy, Clone, clap::ValueEnum)]
pub enum Role {
    Upload,
    Owner,
}
