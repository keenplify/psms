use super::{Engine, Result, Target};
use rbdc::db::Connection;
use rbs::Value;
use serde_json::Value as Json;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn arg(s: &str) -> Value {
    Value::String(s.into())
}
pub async fn rows(c: &mut dyn Connection, sql: &str, args: Vec<Value>) -> Result<Vec<Json>> {
    Ok(rbs::from_value(c.exec_decode(sql, args).await?)?)
}

impl Target {
    pub async fn schema(&self, c: &mut dyn Connection) -> Result<()> {
        let text = match self.engine {
            Engine::Sqlserver => "NVARCHAR(MAX)",
            Engine::Mysql | Engine::Mariadb => "LONGTEXT",
            Engine::Postgres => "TEXT",
        };
        for (name,cols) in [
    ("psms_archive","id BIGINT PRIMARY KEY NOT NULL,revision BIGINT NOT NULL,instance_id VARCHAR(36) NOT NULL".into()),
    ("psms_messages",format!("id VARCHAR(36) PRIMARY KEY NOT NULL,position_id BIGINT NOT NULL UNIQUE,provider VARCHAR(64) NOT NULL,body {text} NOT NULL,archived_at BIGINT NOT NULL,starred BIGINT NOT NULL,contact_id VARCHAR(64)")),
    ("psms_contacts","id VARCHAR(64) PRIMARY KEY NOT NULL,first_seen_at BIGINT NOT NULL,last_seen_at BIGINT NOT NULL".into()),
    ("psms_tombstones","id VARCHAR(36) PRIMARY KEY NOT NULL".into()),
    ("psms_events","revision BIGINT PRIMARY KEY NOT NULL,message_id VARCHAR(36) NOT NULL,event_type VARCHAR(32) NOT NULL".into())
  ]{self.create(c,name,&cols).await?}
        if rows(c, "SELECT id FROM psms_archive WHERE id=1", vec![])
            .await?
            .is_empty()
        {
            c.exec(
                "INSERT INTO psms_archive(id,revision,instance_id) VALUES(1,0,?)",
                vec![arg(&uuid::Uuid::new_v4().to_string())],
            )
            .await?;
        }
        Ok(())
    }
    pub fn limit(&self, sql: &str, limit: usize) -> String {
        if self.engine == Engine::Sqlserver {
            sql.replacen("SELECT ", &format!("SELECT TOP ({limit}) "), 1)
        } else {
            format!("{sql} LIMIT {limit}")
        }
    }
    async fn write(&self) -> Result<Box<dyn Connection>> {
        let mut c = self.open().await?;
        c.begin().await?;
        // Lock just this archive until commit. Separate daemons may share an archive.
        c.exec(
            "UPDATE psms_archive SET revision=revision+1 WHERE id=1",
            vec![],
        )
        .await?;
        Ok(c)
    }
    async fn event(c: &mut dyn Connection, id: &str, kind: &str) -> Result<i64> {
        let r = rows(c, "SELECT revision FROM psms_archive WHERE id=1", vec![]).await?;
        let rev = r
            .first()
            .and_then(|r| r["revision"].as_i64())
            .ok_or("missing archive revision")?;
        c.exec(
            "INSERT INTO psms_events(revision,message_id,event_type) VALUES(?,?,?)",
            vec![rev.into(), arg(id), arg(kind)],
        )
        .await?;
        Ok(rev)
    }
    pub async fn insert(&self, m: &crate::Message) -> Result<u16> {
        let mut c = self.write().await?;
        let id = m.id.to_string();
        let existing = rows(
            c.as_mut(),
            "SELECT provider,body,contact_id FROM psms_messages WHERE id=?",
            vec![arg(&id)],
        )
        .await?;
        if let Some(row) = existing.first() {
            let same = row["provider"] == m.sender
                && row["body"] == m.body
                && row["contact_id"].as_str() == m.contact_id.as_deref();
            c.rollback().await?;
            return Ok(if same { 200 } else { 409 });
        }
        if !rows(
            c.as_mut(),
            "SELECT id FROM psms_tombstones WHERE id=?",
            vec![arg(&id)],
        )
        .await?
        .is_empty()
        {
            c.rollback().await?;
            return Ok(410);
        }
        let time = now();
        let position = Self::event(c.as_mut(), &id, "message.created").await?;
        if let Some(contact) = &m.contact_id {
            if rows(
                c.as_mut(),
                "SELECT id FROM psms_contacts WHERE id=?",
                vec![arg(contact)],
            )
            .await?
            .is_empty()
            {
                c.exec(
                    "INSERT INTO psms_contacts(id,first_seen_at,last_seen_at) VALUES(?,?,?)",
                    vec![arg(contact), time.into(), time.into()],
                )
                .await?;
            } else {
                c.exec(
                    "UPDATE psms_contacts SET last_seen_at=? WHERE id=? AND last_seen_at<?",
                    vec![time.into(), arg(contact), time.into()],
                )
                .await?;
            }
        }
        c.exec("INSERT INTO psms_messages(id,position_id,provider,body,archived_at,starred,contact_id) VALUES(?,?,?,?,?,0,?)",vec![arg(&id),position.into(),arg(&m.sender),arg(&m.body),time.into(),m.contact_id.as_deref().map(arg).unwrap_or(Value::Null)]).await?;
        c.commit().await?;
        Ok(201)
    }
    pub async fn edit(&self, id: &str, star: Option<bool>) -> Result<bool> {
        let mut c = self.write().await?;
        let found = rows(
            c.as_mut(),
            "SELECT starred FROM psms_messages WHERE id=?",
            vec![arg(id)],
        )
        .await?;
        let Some(row) = found.first() else {
            c.rollback().await?;
            return Ok(false);
        };
        if let Some(flag) = star {
            let flag = i64::from(flag);
            if row["starred"].as_i64() == Some(flag) {
                c.rollback().await?;
                return Ok(true);
            }
            c.exec(
                "UPDATE psms_messages SET starred=? WHERE id=?",
                vec![flag.into(), arg(id)],
            )
            .await?;
            Self::event(c.as_mut(), id, "message.updated").await?;
        } else {
            c.exec("DELETE FROM psms_messages WHERE id=?", vec![arg(id)])
                .await?;
            c.exec("INSERT INTO psms_tombstones(id) VALUES(?)", vec![arg(id)])
                .await?;
            Self::event(c.as_mut(), id, "message.deleted").await?;
        }
        c.commit().await?;
        Ok(true)
    }
}
