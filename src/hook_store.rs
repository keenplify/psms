use rusqlite::{Connection, params};
use uuid::Uuid;

pub fn list(db: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    db.prepare("SELECT id,url FROM webhook_endpoints ORDER BY rowid")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}
pub fn valid_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
            && u.query().is_none()
    })
}
pub fn create(db: &Connection, url: &str) -> rusqlite::Result<(String, String)> {
    let id = Uuid::new_v4().to_string();
    let token = secret();
    db.execute(
        "INSERT INTO webhook_endpoints(id,url,bearer) VALUES(?1,?2,?3)",
        params![id, url, token],
    )?;
    Ok((id, token))
}
fn secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
pub fn rotate(db: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    let token = secret();
    let changed = db.execute(
        "UPDATE webhook_endpoints SET bearer=?1 WHERE id=?2",
        params![token, id],
    )?;
    Ok((changed == 1).then_some(token))
}
pub fn remove(db: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(db.execute("DELETE FROM webhook_endpoints WHERE id=?1", [id])? == 1)
}
