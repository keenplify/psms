use std::{
    env, io,
    path::Path,
    process::{Child, Command, Stdio},
};
/// Whether this program runs from inside pSMS.app on a Mac.
pub fn in_app_bundle() -> bool {
    cfg!(target_os = "macos")
        && env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
}
/// Becomes the desktop window in this same process, so pSMS.app shows one Dock icon with its
/// own name and icon instead of a second, unnamed program. Returns only if that failed.
#[cfg(unix)]
pub fn become_window(db: &Path, settings: bool) -> io::Error {
    use std::os::unix::process::CommandExt;
    let mut path = match env::current_exe() {
        Ok(p) => p,
        Err(e) => return e,
    };
    path.set_file_name("psms-desktop");
    Command::new(&path)
        .arg(if settings { "settings" } else { "dashboard" })
        .arg(db)
        .stdin(Stdio::null())
        .exec()
}
pub fn launch(db: &Path, settings: bool) -> io::Result<Child> {
    let mut path = env::current_exe()?;
    path.set_file_name(format!("psms-desktop{}", env::consts::EXE_SUFFIX));
    let mut cmd = Command::new(&path);
    cmd.arg(if settings { "settings" } else { "dashboard" })
        .arg(db)
        .stdin(Stdio::null());
    cmd.spawn().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "Could not open the GPUI Kit desktop application at {}: {e}",
                path.display()
            ),
        )
    })
}
