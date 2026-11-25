use crate::cli::ServerOptions;
use std::{
    env,
    error::Error,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
fn sidecar(db: &Path, suffix: &str) -> PathBuf {
    let mut path = OsString::from(db);
    path.push(suffix);
    path.into()
}
fn latch(db: &Path, suffix: &str) -> Result<File> {
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(sidecar(db, suffix))?)
}
fn active(db: &Path) -> Result<bool> {
    match latch(db, ".daemon.lock")?.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(e) => Err(e.into()),
    }
}
fn url(db: &Path) -> Result<String> {
    let addr = fs::read_to_string(sidecar(db, ".ready"))?
        .trim()
        .parse::<SocketAddr>()?;
    Ok(format!("http://{addr}"))
}
pub fn status(db: &Path) -> Result<()> {
    if active(db)? {
        println!("pSMS is running at {}", url(db)?);
    } else {
        println!("pSMS is stopped.");
    }
    Ok(())
}
pub fn init(db: &Path, options: &ServerOptions) -> Result<()> {
    let control = latch(db, ".init.lock")?;
    control.lock()?;
    if active(db)? {
        return status(db);
    }
    for suffix in [".ready", ".stop"] {
        let path = sidecar(db, suffix);
        if path.try_exists()? {
            fs::remove_file(path)?;
        }
    }
    let log = sidecar(db, ".daemon.log");
    let mut cmd = Command::new(env::current_exe()?);
    cmd.arg("daemon-worker")
        .arg("--database")
        .arg(db)
        .arg("--api-prefix")
        .arg(&options.api_prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(OpenOptions::new().append(true).create(true).open(&log)?);
    if let Some(bind) = options.bind {
        cmd.arg("--bind").arg(bind.to_string());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::{
            Foundation::{HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation},
            System::Console::{
                GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
            },
        };
        // A redirected caller may have supplied inheritable pipes. A detached worker
        // retaining a duplicate prevents tools waiting on CLI output from seeing EOF.
        for stream in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            unsafe {
                let handle = GetStdHandle(stream);
                if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                    SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
                }
            }
        }
        cmd.creation_flags(0x08000000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait()?.is_some() {
            return Err(format!("pSMS could not start. See {}", log.display()).into());
        }
        if active(db)?
            && let Ok(address) = url(db)
        {
            println!(
                "pSMS is running in the background.\nAPI: {address}\nArchive: {}\nUse 'psms desktop' for the dashboard and 'psms settings' for preferences.\nFor iPhone access, use your public HTTPS endpoint or run 'psms tunnel --help'.",
                db.display()
            );
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("timed out waiting for the local server".into());
        }
        thread::sleep(Duration::from_millis(80));
    }
}
pub fn stop(db: &Path) -> Result<()> {
    let control = latch(db, ".init.lock")?;
    control.lock()?;
    if !active(db)? {
        return status(db);
    }
    File::create(sidecar(db, ".stop"))?;
    for _ in 0..100 {
        if !active(db)? {
            println!("pSMS stopped.");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("pSMS is still draining requests; check psms status".into())
}
pub struct Worker {
    db: PathBuf,
    _lock: File,
}
impl Worker {
    pub fn acquire(db: &Path) -> Result<Self> {
        let lock = latch(db, ".daemon.lock")?;
        lock.try_lock()?;
        Ok(Self {
            db: db.into(),
            _lock: lock,
        })
    }
    pub fn ready(&self, addr: SocketAddr) -> Result<()> {
        let temp = sidecar(&self.db, ".ready.tmp");
        {
            let mut f = File::create(&temp)?;
            writeln!(f, "{addr}")?;
        }
        fs::rename(temp, sidecar(&self.db, ".ready"))?;
        Ok(())
    }
    pub fn request_path(&self) -> PathBuf {
        sidecar(&self.db, ".stop")
    }
}
pub async fn await_stop(request: PathBuf) {
    while !request.exists() {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        for suffix in [".ready", ".stop"] {
            let _ = fs::remove_file(sidecar(&self.db, suffix));
        }
    }
}

pub fn running_address(db: &Path) -> Result<Option<SocketAddr>> {
    if !active(db)? {
        return Ok(None);
    }
    Ok(Some(
        fs::read_to_string(sidecar(db, ".ready"))?.trim().parse()?,
    ))
}
