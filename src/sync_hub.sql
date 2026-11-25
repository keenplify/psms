CREATE TABLE IF NOT EXISTS replicated_messages (
 region TEXT NOT NULL, id TEXT NOT NULL, revision INTEGER NOT NULL,
 sender TEXT, body TEXT, archived_at INTEGER, starred INTEGER NOT NULL,
 PRIMARY KEY(region,id)
);
CREATE VIEW IF NOT EXISTS regional_inbox AS
 SELECT 'sg' AS region,m.id,m.sender,m.body,m.archived_at,coalesce(f.starred,0) AS starred
 FROM messages m LEFT JOIN message_flags f ON f.message_id=m.id
 UNION ALL
 SELECT region,id,sender,body,archived_at,starred FROM replicated_messages WHERE body IS NOT NULL;
