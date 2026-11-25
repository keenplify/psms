use super::{
    Result, Target,
    archive::{arg, now, rows},
};
use rbdc::db::Connection;
use rbs::Value;
use serde_json::{Value as Json, json};
const FIELDS: &str = "id,provider AS sender,body,archived_at,starred,position_id";
fn page(mut items: Vec<Json>, limit: usize, key: &str, next: &str) -> Json {
    let more = items.len() > limit;
    items.truncate(limit);
    let cursor = if more {
        items.last().map(|r| r[next].clone())
    } else {
        None
    };
    for row in &mut items {
        if let Some(obj) = row.as_object_mut() {
            obj.remove("position_id");
            if let Some(star) = obj.get_mut("starred") {
                *star = json!(star.as_i64() == Some(1));
            }
        }
    }
    json!({(key):items,"next_cursor":cursor})
}
impl Target {
    pub async fn messages(&self, cursor: i64, limit: usize) -> Result<Json> {
        let mut c = self.open().await?;
        let sql = self.limit(
            &format!(
                "SELECT {FIELDS} FROM psms_messages WHERE position_id<? ORDER BY position_id DESC"
            ),
            limit + 1,
        );
        Ok(page(
            rows(c.as_mut(), &sql, vec![cursor.into()]).await?,
            limit,
            "messages",
            "position_id",
        ))
    }
    pub async fn message(&self, id: &str) -> Result<Option<Json>> {
        let mut c = self.open().await?;
        let sql = format!("SELECT {FIELDS} FROM psms_messages WHERE id=?");
        let mut entries = page(
            rows(c.as_mut(), &sql, vec![arg(id)]).await?,
            1,
            "messages",
            "position_id",
        );
        Ok(entries["messages"].as_array_mut().and_then(Vec::pop))
    }
    async fn sender_page(
        &self,
        c: &mut dyn Connection,
        id: &str,
        cursor: &str,
        limit: usize,
    ) -> Result<Json> {
        let sql = self.limit(
            &format!("SELECT {FIELDS} FROM psms_messages WHERE contact_id=? AND id>? ORDER BY id"),
            limit + 1,
        );
        Ok(page(
            rows(c, &sql, vec![arg(id), arg(cursor)]).await?,
            limit,
            "messages",
            "id",
        ))
    }
    pub async fn contacts(
        &self,
        filter: Option<&str>,
        cursor: &str,
        limit: usize,
        include: bool,
    ) -> Result<Json> {
        let mut c = self.open().await?;
        let mut params = vec![arg(cursor)];
        let mut sql =
            "SELECT id,first_seen_at,last_seen_at FROM psms_contacts WHERE id>?".to_owned();
        if let Some(id) = filter {
            sql.push_str(" AND id=?");
            params.push(arg(id))
        }
        sql.push_str(" ORDER BY id");
        let mut result = page(
            rows(c.as_mut(), &self.limit(&sql, limit + 1), params).await?,
            limit,
            "contacts",
            "id",
        );
        if include {
            for item in result["contacts"]
                .as_array_mut()
                .ok_or("invalid contact listing")?
            {
                let id = item["id"].as_str().ok_or("missing contact id")?;
                item["messages"] = self.sender_page(c.as_mut(), id, "", 5).await?;
            }
        }
        Ok(result)
    }
    pub async fn history(&self, id: &str, cursor: &str, limit: usize) -> Result<Option<Json>> {
        let mut c = self.open().await?;
        if rows(
            c.as_mut(),
            "SELECT id FROM psms_contacts WHERE id=?",
            vec![arg(id)],
        )
        .await?
        .is_empty()
        {
            return Ok(None);
        }
        Ok(Some(self.sender_page(c.as_mut(), id, cursor, limit).await?))
    }
    pub async fn recent(&self) -> Result<Vec<Json>> {
        let mut c = self.open().await?;
        let time = now();
        rows(c.as_mut(),&self.limit("SELECT body,archived_at FROM psms_messages WHERE provider=? AND archived_at BETWEEN ? AND ? ORDER BY position_id DESC",25),vec![arg(crate::encryption::ACTIONS),(time-300).into(),time.into()]).await
    }
    pub async fn revision(&self) -> Result<i64> {
        let mut c = self.open().await?;
        let r = rows(
            c.as_mut(),
            "SELECT revision FROM psms_archive WHERE id=1",
            vec![],
        )
        .await?;
        r.first()
            .and_then(|v| v["revision"].as_i64())
            .ok_or("missing archive revision".into())
    }
    pub async fn events(&self, after: i64) -> Result<Vec<Json>> {
        let mut c = self.open().await?;
        rows(c.as_mut(),&self.limit("SELECT revision,message_id,event_type FROM psms_events WHERE revision>? ORDER BY revision",32),vec![Value::I64(after)]).await
    }
}
