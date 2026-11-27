use plist::Value;
use std::{error::Error, fs, io::Cursor, path::Path};

/// Build on the archive owner's machine. Neither the generated key nor the private workflow is logged.
pub fn prepare(
    endpoint: &str,
    folder: &Path,
    token: &str,
    username: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let uri = endpoint.parse::<axum::http::Uri>()?;
    if uri.scheme_str() != Some("https")
        || uri.host().is_none()
        || uri.authority().is_some_and(|a| a.as_str().contains('@'))
        || uri.query().is_some()
        || !uri.path().ends_with("/messages")
    {
        return Err("supply a full HTTPS endpoint ending in /messages".into());
    }
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "could not generate archive key")?;
    let key = bytes.iter().map(|n| format!("{n:02x}")).collect::<String>();
    let mut workflow = Value::from_reader(Cursor::new(include_bytes!(
        "../examples/Papamo SMS Encrypted.shortcut"
    )))?;
    let root = workflow.as_dictionary_mut().ok_or("invalid workflow")?;
    let actions = root
        .get_mut("WFWorkflowActions")
        .and_then(Value::as_array_mut)
        .ok_or("missing actions")?;
    for (index, text) in [endpoint, token, &key].into_iter().enumerate() {
        let fields = actions
            .get_mut(index)
            .and_then(Value::as_dictionary_mut)
            .and_then(|a| a.get_mut("WFWorkflowActionParameters"))
            .and_then(Value::as_dictionary_mut)
            .ok_or("missing setup field")?;
        fields.insert("WFTextActionText".into(), text.into());
    }
    root.insert("WFWorkflowImportQuestions".into(), Value::Array(vec![]));
    let mut dir = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        dir.mode(0o700);
    }
    #[cfg(not(unix))]
    dir.recursive(false);
    dir.create(folder)?;
    fs::write(
        folder.join("pSMS-Recovery-Kit.pdf"),
        crate::recovery::printable(
            &bytes,
            uri.authority().ok_or("missing host")?.as_str(),
            username,
        ),
    )?;
    fs::write(
        folder.join("pSMS.json"),
        serde_json::to_vec(&serde_json::json!({"endpoint":endpoint,"token":token,"key":key}))?,
    )?;
    workflow.to_file_binary(folder.join("pSMS_Encrypted.shortcut"))?;
    fs::write(folder.join("archive-recovery-key.txt"), format!("{key}\n"))?;
    #[cfg(target_os = "macos")]
    if std::env::var_os("CI").is_none()
        && std::env::var_os("SSH_CONNECTION").is_none()
        && let Err(error) = open_mobile_template(folder)
    {
        eprintln!("Saved pairing files. Could not open the public template in Shortcuts: {error}");
    }
    println!(
        "Private shortcut, printable recovery kit and recovery key saved in {}. Keep the folder private. Import the shared pSMS Capture shortcut, paste the private phone setup from pSMS.json and enable the Message automation.",
        folder.display()
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_mobile_template(folder: &Path) -> std::io::Result<()> {
    use std::process::Command;
    let template = folder.join("pSMS-template.shortcut");
    let signed = folder.join("pSMS-Capture.shortcut");
    fs::write(
        &template,
        include_bytes!("../examples/pSMS-Capture.shortcut"),
    )?;
    if !Command::new("/usr/bin/shortcuts")
        .args(["sign", "--mode", "anyone", "--input"])
        .arg(&template)
        .arg("--output")
        .arg(&signed)
        .status()?
        .success()
    {
        return Err(std::io::Error::other("Apple Shortcuts signing failed"));
    }
    // Automated runs (tests, release checks) sign the shortcut without opening Shortcuts.
    if std::env::var_os("PSMS_ONBOARD_NO_OPEN").is_some() {
        return Ok(());
    }
    if !Command::new("/usr/bin/open")
        .args(["-a", "Shortcuts"])
        .arg(&signed)
        .status()?
        .success()
    {
        return Err(std::io::Error::other(
            "Shortcuts could not open the signed template",
        ));
    }
    Ok(())
}
