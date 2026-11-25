use std::{
    env, io,
    path::Path,
    process::{Child, Command, Stdio},
};
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
