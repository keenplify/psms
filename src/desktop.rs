use rusqlite::Connection;
use uuid::Uuid;

pub fn secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn credentials(db: &mut Connection) -> rusqlite::Result<(String, String)> {
    let tx = db.transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS local_credentials (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1), upload TEXT NOT NULL, owner TEXT NOT NULL)")?;
    tx.execute(
        "INSERT OR IGNORE INTO local_credentials VALUES (1, ?1, ?2)",
        [secret(), secret()],
    )?;
    let values = tx.query_row(
        "SELECT upload,owner FROM local_credentials WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    tx.commit()?;
    Ok(values)
}
