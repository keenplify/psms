//! Opening pSMS when you sign in to the computer, which is off until you turn it on. A login
//! launch passes `--at-login`, so pSMS checks for an update straight away and installs it, and
//! new texts notify without anyone opening the app.

use std::path::PathBuf;

/// The argument the login item passes, and the variable that carries it to the window.
pub const ARG: &str = "--at-login";
pub const ENV: &str = "PSMS_AT_LOGIN";

/// Whether this window was opened by the login item.
pub fn launched_at_login() -> bool {
    std::env::var_os(ENV).is_some()
}

fn app() -> Result<PathBuf, &'static str> {
    std::env::current_exe().map_err(|_| "Could not find the pSMS app.")
}

#[cfg(target_os = "macos")]
fn agent() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Library/LaunchAgents/dev.papamo.psms.plist"))
}
#[cfg(target_os = "macos")]
pub fn enabled() -> bool {
    agent().is_some_and(|p| p.exists())
}
#[cfg(target_os = "macos")]
pub fn set(on: bool) -> Result<(), &'static str> {
    let agent = agent().ok_or("Could not find your home folder.")?;
    if !on {
        return match std::fs::remove_file(&agent) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err("Could not turn off opening at login.")
            }
            _ => Ok(()),
        };
    }
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    // Open pSMS.app through macOS, so the login launch shows the app's own Dock icon; a build
    // outside an app bundle runs directly.
    let exe = app()?;
    let program = match exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
    {
        Some(bundle) => format!(
            "<string>/usr/bin/open</string><string>-a</string><string>{}</string><string>--args</string><string>{ARG}</string>",
            escape(&bundle.to_string_lossy())
        ),
        None => format!(
            "<string>{}</string><string>{ARG}</string>",
            escape(&exe.to_string_lossy())
        ),
    };
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>dev.papamo.psms</string>
<key>ProgramArguments</key><array>{program}</array>
<key>RunAtLoad</key><true/>
<key>ProcessType</key><string>Interactive</string>
</dict></plist>
"#
    );
    std::fs::create_dir_all(agent.parent().ok_or("Could not open LaunchAgents.")?)
        .and_then(|_| std::fs::write(&agent, plist))
        .map_err(|_| "Could not turn on opening at login.")
}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
fn reg(args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg")
        .args(args)
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .is_ok_and(|o| o.status.success())
}
#[cfg(windows)]
pub fn enabled() -> bool {
    reg(&["query", RUN_KEY, "/v", "pSMS"])
}
#[cfg(windows)]
pub fn set(on: bool) -> Result<(), &'static str> {
    if !on {
        return if !enabled() || reg(&["delete", RUN_KEY, "/v", "pSMS", "/f"]) {
            Ok(())
        } else {
            Err("Could not turn off opening at login.")
        };
    }
    let command = format!("\"{}\" {ARG}", app()?.display());
    if reg(&[
        "add", RUN_KEY, "/v", "pSMS", "/t", "REG_SZ", "/d", &command, "/f",
    ]) {
        Ok(())
    } else {
        Err("Could not turn on opening at login.")
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn entry() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|c| c.join("autostart/dev.papamo.psms.desktop"))
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn enabled() -> bool {
    entry().is_some_and(|p| p.exists())
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn set(on: bool) -> Result<(), &'static str> {
    let entry = entry().ok_or("Could not find your home folder.")?;
    if !on {
        return match std::fs::remove_file(&entry) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err("Could not turn off opening at login.")
            }
            _ => Ok(()),
        };
    }
    let app = app()?.to_string_lossy().replace('"', "\\\"");
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=pSMS\nComment=Your encrypted SMS inbox\nExec=\"{app}\" {ARG}\nIcon=psms\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    );
    std::fs::create_dir_all(entry.parent().ok_or("Could not open autostart.")?)
        .and_then(|_| std::fs::write(&entry, desktop))
        .map_err(|_| "Could not turn on opening at login.")
}
