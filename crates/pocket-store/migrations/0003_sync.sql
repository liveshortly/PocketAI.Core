-- NET-22: replicated chat op log. Every conversation/message change is an op;
-- the conversations/messages tables are the materialized view of these ops.
CREATE TABLE sync_ops (op_id TEXT PRIMARY KEY, device_id TEXT NOT NULL, seq INTEGER NOT NULL,
  lamport INTEGER NOT NULL, entity TEXT NOT NULL, entity_id TEXT NOT NULL, conv_id TEXT NOT NULL,
  field TEXT NOT NULL, value TEXT NOT NULL, ts INTEGER NOT NULL);
CREATE INDEX sync_ops_order ON sync_ops(lamport, device_id);
CREATE INDEX sync_ops_entity ON sync_ops(entity_id, field);
CREATE INDEX sync_ops_conv ON sync_ops(conv_id, entity);
CREATE INDEX sync_ops_origin ON sync_ops(device_id, seq);
-- Version vector: highest op seq this replica holds from each origin device.
CREATE TABLE sync_state (device_id TEXT PRIMARY KEY, max_seq INTEGER NOT NULL);
-- Content-addressed attachments; path NULL = referenced but not fetched yet.
CREATE TABLE sync_blobs (sha256 TEXT PRIMARY KEY, path TEXT);
ALTER TABLE messages ADD COLUMN lamport INTEGER NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN sibling_of TEXT;
