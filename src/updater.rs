use base64::{Engine, engine::general_purpose::STANDARD};
use clap::Args;
use ring::signature::{ED25519, UnparsedPublicKey};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    error::Error,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const REPO: &str = "https://github.com/keenplify/psms/releases/download";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Args)]
pub struct Options {
    /// Report the latest stable version without downloading it.
    #[arg(long, conflicts_with = "prepare")]
    check: bool,
    /// Download and verify, leaving the active version unchanged.
    #[arg(long)]
    prepare: bool,
    #[arg(long,hide=true, conflicts_with_all=["check","prepare"])]
    restart: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    files: HashMap<String, String>,
}
#[cfg(unix)]
fn elevated() -> bool {
    unsafe { libc::geteuid() == 0 || libc::geteuid() != libc::getuid() }
}
#[cfg(windows)]
fn elevated() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}
fn root() -> Result<PathBuf> {
    if elevated() {
        return Err("Run user updates from a non-elevated account. System services use the regional rollout.".into());
    }
    Ok(dirs::data_local_dir()
        .ok_or("No local data directory")?
        .join("papamo-sms")
        .join("updates")
        .join(platform()?))
}
fn platform() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("macos-arm64"),
        ("windows", "x86_64") => Ok("windows-x64"),
        ("linux", "x86_64") => Ok("linux-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        _ => Err("No update package for this platform".into()),
    }
}
fn stable(s: &str) -> Result<Version> {
    let v = Version::parse(s)?;
    if !(v.pre.is_empty() && v.build.is_empty()) || v.to_string() != s {
        return Err("Expected a stable release".into());
    }
    Ok(v)
}
fn active(root: &Path) -> Result<Option<Version>> {
    match fs::read_to_string(root.join("active")) {
        Ok(s) => Ok(Some(stable(s.trim())?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn executable(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}
/// All launchers select a complete version directory through one atomic pointer.
pub fn dispatch() -> Result<()> {
    let Ok(root) = root() else { return Ok(()) };
    let Some(v) = active(&root)? else {
        return Ok(());
    };
    if v <= stable(CURRENT)? {
        return Ok(());
    }
    let mut child = Command::new(root.join(v.to_string()).join(executable("psms")));
    child.args(std::env::args_os().skip(1));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(child.exec().into())
    }
    #[cfg(not(unix))]
    {
        std::process::exit(child.status()?.code().unwrap_or(1))
    }
}
fn verified_manifest(data: &[u8], sig: &[u8], version: &Version) -> Result<Manifest> {
    let key = include_str!("../packaging/update-public.pem")
        .lines()
        .filter(|l| !l.starts_with('-'))
        .collect::<String>();
    let der = STANDARD.decode(key)?;
    let pubkey = der
        .strip_prefix(&[
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ])
        .ok_or("Invalid release key")?;
    UnparsedPublicKey::new(&ED25519, pubkey)
        .verify(data, sig)
        .map_err(|_| "Release signature verification failed")?;
    let manifest: Manifest = serde_json::from_slice(data)?;
    if stable(&manifest.version)? != *version {
        return Err("Release version mismatch".into());
    }
    Ok(manifest)
}
async fn fetch(http: &reqwest::Client, url: String, mut write: impl Write, max: u64) -> Result<()> {
    let mut res = http.get(url).send().await?.error_for_status()?;
    let mut count = 0;
    while let Some(chunk) = res.chunk().await? {
        count += chunk.len() as u64;
        if count > max {
            return Err("Update exceeds size limit".into());
        }
        write.write_all(&chunk)?;
    }
    Ok(())
}
fn digest(path: &Path) -> Result<String> {
    let mut h = Sha256::new();
    let mut file = fs::File::open(path)?;
    let mut buf = [0; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn write_version(root: &Path, name: &str, version: &Version) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    write!(file, "{version}")?;
    file.as_file().sync_all()?;
    file.persist(root.join(name))?;
    Ok(())
}

// Before opening a window, activate the already verified download offline.
// dispatch() then forwards the original desktop/settings arguments to the new CLI.
pub async fn activate_prepared(db: &Path, server: &crate::cli::ServerOptions) -> Result<()> {
    let Ok(root) = root() else { return Ok(()) };
    let raw = match fs::read_to_string(root.join("pending")) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let version = stable(raw.trim())?;
    if version > stable(CURRENT)? {
        apply(
            Options {
                check: false,
                prepare: false,
                restart: Some(version.to_string()),
            },
            db,
            server,
            false,
        )
        .await?;
        dispatch()?;
    }
    Ok(())
}
pub async fn run(opts: Options, db: &Path, server: &crate::cli::ServerOptions) -> Result<()> {
    apply(opts, db, server, true).await
}
async fn apply(
    opts: Options,
    db: &Path,
    server: &crate::cli::ServerOptions,
    reopen: bool,
) -> Result<()> {
    let root = root()?;
    fs::create_dir_all(&root)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("update.lock"))?;
    lock.try_lock().map_err(|_| "Another update is running")?;
    let http = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(8))
        .user_agent("papamo-psms-updater")
        .build()?;
    let version = if let Some(v) = &opts.restart {
        stable(v)?
    } else {
        let mut latest = Vec::new();
        fetch(
            &http,
            "https://api.github.com/repos/keenplify/psms/releases/latest".into(),
            &mut latest,
            512 * 1024,
        )
        .await?;
        let json: serde_json::Value = serde_json::from_slice(&latest)?;
        stable(
            json["tag_name"]
                .as_str()
                .and_then(|s| s.strip_prefix('v'))
                .ok_or("Invalid release tag")?,
        )?
    };
    if version <= stable(CURRENT)? || active(&root)?.is_some_and(|v| v >= version) {
        if !opts.prepare {
            println!("pSMS is up to date ({CURRENT}).")
        }
        return Ok(());
    }
    let prefix = format!("{REPO}/v{version}/");
    let mut manifest = Vec::new();
    let mut sig = Vec::new();
    let target = root.join(version.to_string());
    if opts.restart.is_some() {
        manifest = fs::read(target.join("update.json"))?;
        sig = fs::read(target.join("update.sig"))?;
    } else {
        fetch(&http, format!("{prefix}update.json"), &mut manifest, 65536).await?;
        fetch(&http, format!("{prefix}update.sig"), &mut sig, 64).await?;
    }
    let metadata = verified_manifest(&manifest, &sig, &version)?;
    if opts.check {
        println!("pSMS {version} is available.");
        return Ok(());
    }
    let temp = tempfile::tempdir_in(&root)?;
    for name in ["psms", "psms-desktop"] {
        let asset = executable(&format!("{name}-{}", platform()?));
        let expected = metadata
            .files
            .get(&asset)
            .ok_or("Missing asset in signed release")?;
        let file = executable(name);
        if target.exists() {
            if digest(&target.join(file))? != *expected {
                return Err("Cached update failed verification".into());
            }
            continue;
        }
        let path = temp.path().join(file);
        let mut f = fs::File::create(&path)?;
        fetch(&http, format!("{prefix}{asset}"), &mut f, 300 * 1024 * 1024).await?;
        f.sync_all()?;
        drop(f);
        if digest(&path)? != *expected {
            return Err("Downloaded update failed verification".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
    }
    if !target.exists() {
        fs::write(temp.path().join("update.json"), manifest)?;
        fs::write(temp.path().join("update.sig"), sig)?;
        fs::rename(temp.path(), &target)?;
    }
    if opts.prepare {
        write_version(&root, "pending", &version)?;
        println!("{version}");
        return Ok(());
    }
    let running = crate::daemon::running_address(db)?.is_some();
    let mut marker = tempfile::NamedTempFile::new_in(&root)?;
    write!(marker, "{version}")?;
    marker.as_file().sync_all()?;
    if running {
        crate::daemon::stop(db)?;
    }
    if let Err(e) = marker.persist(root.join("active")) {
        if running {
            crate::daemon::init(db, server)?;
        }
        return Err(e.into());
    }
    let restart_window = reopen && opts.restart.is_some();
    // pSMS.app reopens through macOS, so the Dock shows one pSMS: after a moment, once this
    // window has quit and released the one-window lock.
    #[cfg(target_os = "macos")]
    if restart_window && crate::native::in_app_bundle() {
        if running {
            Command::new(target.join(executable("psms")))
                .arg("--database")
                .arg(db)
                .arg("init")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()?;
        }
        Command::new("/bin/sh")
            .args([
                "-c",
                "sleep 2; /usr/bin/open -b \"$0\"",
                crate::native::BUNDLE_ID,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        println!("Updated pSMS to {version}.");
        return Ok(());
    }
    if restart_window || running {
        let cmd = if restart_window { "desktop" } else { "init" };
        let status = Command::new(target.join(executable("psms")))
            .arg("--database")
            .arg(db)
            .arg(cmd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        if !status.success() {
            return Err("Update installed; could not restart pSMS".into());
        }
    }
    println!("Updated pSMS to {version}.");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparing_an_update_does_not_switch_a_running_installation() {
        let root = tempfile::tempdir().unwrap();
        let old = stable("0.1.14").unwrap();
        let new = stable("0.1.16").unwrap();
        write_version(root.path(), "active", &old).unwrap();
        write_version(root.path(), "pending", &stable("0.1.15").unwrap()).unwrap();
        write_version(root.path(), "pending", &new).unwrap();
        assert_eq!(active(root.path()).unwrap(), Some(old));
        assert_eq!(
            fs::read_to_string(root.path().join("pending")).unwrap(),
            new.to_string()
        );
    }
    #[test]
    fn rejects_invalid_release_and_signature() {
        assert!(stable("../1.0.0").is_err());
        assert!(stable("1.0.0-preview").is_err());
        assert!(verified_manifest(b"{}", &[0; 64], &stable("1.0.0").unwrap()).is_err());
    }
}
