CREATE TABLE IF NOT EXISTS sync_schedule (
 id INTEGER PRIMARY KEY CHECK(id=1), completed_at INTEGER NOT NULL, region TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_queue (
 seq INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT NOT NULL UNIQUE
);
CREATE TRIGGER IF NOT EXISTS sync_message_insert AFTER INSERT ON messages BEGIN
 DELETE FROM sync_queue WHERE message_id=NEW.id;
 INSERT INTO sync_queue(message_id) VALUES(NEW.id);
END;
CREATE TRIGGER IF NOT EXISTS sync_message_delete AFTER DELETE ON messages BEGIN
 DELETE FROM sync_queue WHERE message_id=OLD.id;
 INSERT INTO sync_queue(message_id) VALUES(OLD.id);
END;
CREATE TRIGGER IF NOT EXISTS sync_flag_insert AFTER INSERT ON message_flags BEGIN
 DELETE FROM sync_queue WHERE message_id=NEW.message_id;
 INSERT INTO sync_queue(message_id) VALUES(NEW.message_id);
END;
CREATE TRIGGER IF NOT EXISTS sync_flag_update AFTER UPDATE ON message_flags BEGIN
 DELETE FROM sync_queue WHERE message_id=NEW.message_id;
 INSERT INTO sync_queue(message_id) VALUES(NEW.message_id);
END;
