use super::{
    Engine, Result, Target,
    archive::{arg, rows},
};
use rbdc::db::Connection as Network;
use rbs::Value;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Serialize, Deserialize)]
pub struct Selection {
    engine: Engine,
    connection_file: PathBuf,
    instance: String,
}
pub fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut out = path.as_os_str().to_os_string();
    out.push(suffix);
    out.into()
}
pub fn local_state(path: &Path) -> PathBuf {
    if sidecar(path, ".storage.json").is_file() {
        sidecar(path, ".settings.sqlite3")
    } else {
        path.into()
    }
}
pub fn lock(path: &Path) -> Result<fs::File> {
    let f = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(sidecar(path, ".serve.lock"))?;
    f.try_lock()
        .map_err(|_| "archive service is active; stop the service before switching storage")?;
    Ok(f)
}
pub async fn runtime(path: &Path) -> Result<Option<Target>> {
    let selected = sidecar(path, ".storage.json");
    if !selected.try_exists()? {
        return Ok(None);
    }
    let cfg: Selection = serde_json::from_slice(&fs::read(selected)?)?;
    let target = Target::load(cfg.engine, &cfg.connection_file)?;
    let mut c = target.open().await?;
    let meta = rows(
        c.as_mut(),
        "SELECT instance_id FROM psms_archive WHERE id=1",
        vec![],
    )
    .await?;
    if meta.first().and_then(|v| v["instance_id"].as_str()) != Some(&cfg.instance) {
        return Err("destination archive identity mismatch".into());
    }
    Ok(Some(target))
}
fn new_file(path: &Path) -> Result<fs::File> {
    let mut opt = fs::OpenOptions::new();
    opt.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opt.mode(0o600);
    }
    Ok(opt.open(path)?)
}
async fn copy_rows(
    db: &Connection,
    c: &mut dyn Network,
    table: &str,
    columns: &[&str],
    select: &str,
) -> Result<usize> {
    let sql = format!(
        "INSERT INTO {table}({}) VALUES({})",
        columns.join(","),
        vec!["?"; columns.len()].join(",")
    );
    let mut stmt = db.prepare(select)?;
    let mut source = stmt.query([])?;
    let mut count = 0;
    while let Some(row) = source.next()? {
        let mut values = Vec::new();
        for col in 0..columns.len() {
            let value = match row.get_ref(col)? {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(n) => Value::I64(n),
                rusqlite::types::ValueRef::Text(t) => arg(std::str::from_utf8(t)?),
                _ => return Err("unexpected archive data type".into()),
            };
            values.push(value)
        }
        c.exec(&sql, values.clone()).await?;
        let fetched = rows(
            c,
            &format!("SELECT {} FROM {table} WHERE id=?", columns.join(",")),
            vec![values[0].clone()],
        )
        .await?;
        if fetched.len() != 1 {
            return Err("destination read-back failed".into());
        }
        for (i, col) in columns.iter().enumerate() {
            let expected = serde_json::to_value(&values[i])?;
            if fetched[0][*col] != expected {
                return Err(format!("destination verification failed for {table}.{col}").into());
            }
        }
        count += 1;
    }
    Ok(count)
}
pub async fn switch(path: &Path, engine: Engine, file: &Path) -> Result<()> {
    let file = file.canonicalize()?;
    let target = Target::load(engine, &file)?;
    target.verify().await?;
    let _guard = lock(path)?;
    if sidecar(path, ".storage.json").try_exists()? {
        return Err("an external archive is already selected".into());
    }
    let mut c = target.open().await?;
    target.schema(c.as_mut()).await?;
    let mut db = if path.try_exists()? {
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?
    } else {
        Connection::open_in_memory()?
    };
    let src = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let migrated = src.table_exists(None, "messages")?;
    let state = sidecar(path, ".settings.sqlite3");
    let staging = sidecar(path, &format!(".{}.staging", uuid::Uuid::new_v4()));
    if state.try_exists()? {
        return Err("existing settings database requires review before switching".into());
    }
    new_file(&staging)?;
    // Only configuration is retained locally. Copy control tables explicitly.
    let prefs = Connection::open(&staging)?;
    for table in [
        "local_credentials",
        "desktop_setup",
        "desktop_preferences",
        "desktop_appearance",
        "app_tokens",
        "sync_interval",
        "webhook_endpoints",
        "webhook_deliveries",
        "sync_schedule",
        "sync_queue",
    ] {
        if !src.table_exists(None, table)? {
            continue;
        }
        let create: String = src.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |r| r.get(0),
        )?;
        prefs.execute_batch(&create)?;
        let mut stmt = src.prepare(&format!("SELECT * FROM {table}"))?;
        let columns = stmt.column_count();
        let sql = format!(
            "INSERT INTO {table} VALUES({})",
            vec!["?"; columns].join(",")
        );
        let mut items = stmt.query([])?;
        while let Some(row) = items.next()? {
            let vals = (0..columns)
                .map(|i| row.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            prefs.execute(&sql, rusqlite::params_from_iter(vals))?;
        }
    }
    drop(prefs);
    c.begin().await?;
    c.exec(
        "UPDATE psms_archive SET revision=revision+1 WHERE id=1",
        vec![],
    )
    .await?;
    if !rows(
        c.as_mut(),
        "SELECT instance_id FROM psms_archive WHERE id=1 AND revision=1",
        vec![],
    )
    .await?
    .is_empty()
    {
    } else {
        return Err("destination is already in use; choose a fresh pSMS database".into());
    }
    let mut max_position = 1;
    if migrated {
        if src.table_exists(None, "replicated_messages")?
            && src.query_row(
                "SELECT EXISTS(SELECT 1 FROM replicated_messages)",
                [],
                |r| r.get::<_, bool>(0),
            )?
        {
            return Err("this source contains replica data; source retained".into());
        }
        if src.table_exists(None, "contacts")? {
            copy_rows(
                &src,
                c.as_mut(),
                "psms_contacts",
                &["id", "first_seen_at", "last_seen_at"],
                "SELECT id,first_seen_at,last_seen_at FROM contacts",
            )
            .await?;
        }
        let cid = if src.table_exists(None, "message_contacts")? {
            "(SELECT contact_id FROM message_contacts WHERE message_id=m.id)"
        } else {
            "NULL"
        };
        copy_rows(&src,c.as_mut(),"psms_messages",&["id","position_id","provider","body","archived_at","starred","contact_id"],&format!("SELECT m.id,m.rowid,m.sender,m.body,m.archived_at,coalesce(f.starred,0),{cid} FROM messages m LEFT JOIN message_flags f ON f.message_id=m.id")).await?;
        copy_rows(
            &src,
            c.as_mut(),
            "psms_tombstones",
            &["id"],
            "SELECT id FROM deleted_messages",
        )
        .await?;
        max_position = src.query_row(
            "SELECT max(1,coalesce(max(rowid),0)) FROM messages",
            [],
            |r| r.get(0),
        )?;
    }
    c.exec(
        "UPDATE psms_archive SET revision=? WHERE id=1",
        vec![Value::I64(max_position)],
    )
    .await?;
    let identity = rows(
        c.as_mut(),
        "SELECT instance_id FROM psms_archive WHERE id=1",
        vec![],
    )
    .await?[0]["instance_id"]
        .as_str()
        .ok_or("missing archive identity")?
        .to_owned();
    c.commit().await?;
    let mut fresh = target.open().await?;
    let meta = rows(
        fresh.as_mut(),
        "SELECT revision FROM psms_archive WHERE instance_id=?",
        vec![arg(&identity)],
    )
    .await?;
    if meta.first().and_then(|v| v["revision"].as_i64()) != Some(max_position) {
        return Err("destination did not pass the post-commit reconnect check".into());
    }
    let selected = sidecar(path, ".storage.json");
    let tmp = sidecar(&staging, ".json");
    let mut f = new_file(&tmp)?;
    f.write_all(&serde_json::to_vec(&Selection {
        engine,
        connection_file: file,
        instance: identity,
    })?)?;
    f.sync_all()?;
    drop(f);
    fs::rename(staging, state)?;
    fs::rename(tmp, selected)?;
    src.commit()?;
    println!(
        "Destination committed and verified on a new connection. Start pSMS and verify /readyz before retiring the previous SQLite archive. The source file was retained."
    );
    Ok(())
}
