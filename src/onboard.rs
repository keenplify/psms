use plist::Value;
use std::{error::Error, fs, io::Cursor, path::Path};

/// Build on the archive owner's machine. Neither the generated key nor the private workflow is logged.
pub fn prepare(endpoint: &str, folder: &Path, token: &str) -> Result<(), Box<dyn Error>> {
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
    workflow.to_file_binary(folder.join("pSMS_Encrypted.shortcut"))?;
    fs::write(folder.join("archive-recovery-key.txt"), format!("{key}\n"))?;
    println!(
        "Private shortcut and recovery key saved in {}. Keep the folder private. Sign the shortcut on a Mac, add it on the phone, then enable its Message automation.",
        folder.display()
    );
    Ok(())
}
