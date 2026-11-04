-- AGT-12: per-chat Tools switch (web search). Local to this device, not synced.
ALTER TABLE conversations ADD COLUMN tools_on INTEGER NOT NULL DEFAULT 0;
