CREATE TABLE IF NOT EXISTS webhook_endpoints(
 id TEXT PRIMARY KEY NOT NULL,url TEXT NOT NULL UNIQUE,bearer TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS webhook_deliveries(
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 endpoint TEXT NOT NULL REFERENCES webhook_endpoints(id) ON DELETE CASCADE,
 message_id TEXT NOT NULL,event TEXT NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,
 due INTEGER NOT NULL DEFAULT(unixepoch())
);
CREATE INDEX IF NOT EXISTS webhook_due ON webhook_deliveries(due,id);
CREATE TRIGGER IF NOT EXISTS webhook_message_created AFTER INSERT ON messages BEGIN
 INSERT INTO webhook_deliveries(endpoint,message_id,event) SELECT id,NEW.id,'message.created' FROM webhook_endpoints;
END;
CREATE TRIGGER IF NOT EXISTS webhook_message_deleted AFTER DELETE ON messages BEGIN
 INSERT INTO webhook_deliveries(endpoint,message_id,event) SELECT id,OLD.id,'message.deleted' FROM webhook_endpoints;
END;
CREATE TRIGGER IF NOT EXISTS webhook_flag_added AFTER INSERT ON message_flags BEGIN
 INSERT INTO webhook_deliveries(endpoint,message_id,event) SELECT id,NEW.message_id,'message.updated' FROM webhook_endpoints;
END;
CREATE TRIGGER IF NOT EXISTS webhook_flag_updated AFTER UPDATE OF starred ON message_flags WHEN OLD.starred<>NEW.starred BEGIN
 INSERT INTO webhook_deliveries(endpoint,message_id,event) SELECT id,NEW.message_id,'message.updated' FROM webhook_endpoints;
END;
