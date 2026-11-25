use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;
pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS app_tokens(scope TEXT PRIMARY KEY NOT NULL,digest BLOB NOT NULL,expires INTEGER)";
pub fn issue(db: &Connection, scope: &str, days: Option<u32>) -> rusqlite::Result<String> {
    let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    db.execute("INSERT INTO app_tokens(scope,digest,expires) VALUES(?1,?2,unixepoch()+?3*86400) ON CONFLICT(scope) DO UPDATE SET digest=excluded.digest,expires=excluded.expires",params![scope,Sha256::digest(secret.as_bytes()).as_slice(),days])?;
    Ok(secret)
}
pub fn revoke(db: &Connection, scope: &str) -> rusqlite::Result<()> {
    db.execute("DELETE FROM app_tokens WHERE scope=?1", [scope])?;
    Ok(())
}
pub fn status(db: &Connection, scope: &str) -> rusqlite::Result<Option<String>> {
    db.query_row("SELECT CASE WHEN expires IS NULL THEN 'Active · no expiration' WHEN expires<=unixepoch() THEN 'Expired' ELSE 'Expires '||datetime(expires,'unixepoch','localtime') END FROM app_tokens WHERE scope=?1",[scope],|r|r.get(0)).optional()
}
pub fn accepts(db: &Connection, scope: &str, token: &[u8]) -> rusqlite::Result<bool> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM app_tokens WHERE scope=?1 AND digest=?2 AND (expires IS NULL OR expires>unixepoch()))",params![scope,Sha256::digest(token).as_slice()],|r|r.get(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotation_expiration_revocation_and_scopes() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(SCHEMA).unwrap();
        let first = issue(&db, "api", None).unwrap();
        let browser = issue(&db, "browser", Some(7)).unwrap();
        assert!(accepts(&db, "api", first.as_bytes()).unwrap());
        assert!(!accepts(&db, "api", browser.as_bytes()).unwrap());
        let second = issue(&db, "api", Some(30)).unwrap();
        assert!(!accepts(&db, "api", first.as_bytes()).unwrap());
        assert!(accepts(&db, "api", second.as_bytes()).unwrap());
        db.execute(
            "UPDATE app_tokens SET expires=unixepoch() WHERE scope='api'",
            [],
        )
        .unwrap();
        assert!(!accepts(&db, "api", second.as_bytes()).unwrap());
        revoke(&db, "browser").unwrap();
        assert!(!accepts(&db, "browser", browser.as_bytes()).unwrap());
    }
}
