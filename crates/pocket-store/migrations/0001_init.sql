CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE models (
  id TEXT PRIMARY KEY, manifest_json TEXT NOT NULL, state TEXT NOT NULL,
  bytes_done INTEGER NOT NULL DEFAULT 0, installed_at INTEGER, last_used_at INTEGER,
  pinned_engine TEXT, benchmark_json TEXT);
CREATE TABLE conversations (id TEXT PRIMARY KEY, title TEXT, project_id TEXT, model_id TEXT,
  system_prompt TEXT, params_json TEXT, created_at INTEGER, updated_at INTEGER, archived INTEGER DEFAULT 0);
CREATE TABLE messages (id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  role TEXT NOT NULL, content_json TEXT NOT NULL, reasoning TEXT, stats_json TEXT, model_id TEXT,
  created_at INTEGER NOT NULL, status TEXT NOT NULL);
CREATE INDEX messages_conv ON messages(conversation_id, created_at);
CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT, instructions TEXT, created_at INTEGER);
CREATE TABLE documents (id TEXT PRIMARY KEY, project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT, mime TEXT, bytes INTEGER, path TEXT, text_chars INTEGER, embed_model TEXT, state TEXT, created_at INTEGER);
CREATE TABLE chunks (id TEXT PRIMARY KEY, document_id TEXT REFERENCES documents(id) ON DELETE CASCADE,
  ord INTEGER, text TEXT, page INTEGER, embedding BLOB);
CREATE VIRTUAL TABLE chunks_fts USING fts5(text, content='chunks', content_rowid='rowid');
CREATE TABLE devices (id TEXT PRIMARY KEY, name TEXT, platform TEXT, public_key BLOB, token_hash BLOB,
  role TEXT, paired_at INTEGER, last_seen_at INTEGER, last_addr TEXT, revoked_at INTEGER);
CREATE TABLE peers (id TEXT PRIMARY KEY, name TEXT, cert_sha256 BLOB, addrs_json TEXT, port INTEGER,
  token_ref TEXT, paired_at INTEGER, last_seen_at INTEGER);
CREATE TABLE audit (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER, actor TEXT, action TEXT, target TEXT,
  detail_json TEXT, prev_hash BLOB, hash BLOB);
CREATE TABLE egress (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER, host TEXT, purpose TEXT, bytes_out INTEGER, bytes_in INTEGER, initiated_by TEXT);
