use std::{
    env, io,
    path::Path,
    process::{Child, Command, Stdio},
};
/// The bundle ID macOS gives pSMS.app.
#[cfg(unix)]
pub const BUNDLE_ID: &str = "dev.papamo.psms";
/// Whether macOS launched this as pSMS.app. Updated versions run from pSMS's data folder, not
/// the bundle, so this follows the bundle ID macOS sets for an app it opens, which carries
/// through the launcher's hand-off to the active version.
#[cfg(unix)]
pub fn in_app_bundle() -> bool {
    cfg!(target_os = "macos")
        && (env::var("__CFBundleIdentifier").is_ok_and(|id| id == BUNDLE_ID)
            || env::current_exe()
                .is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/")))
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
