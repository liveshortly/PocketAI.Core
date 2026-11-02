CREATE TABLE agent_events (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, session_id TEXT NOT NULL,
  approval_id TEXT, kind TEXT NOT NULL, detail_json TEXT NOT NULL);
CREATE INDEX agent_events_session ON agent_events(session_id, id);
