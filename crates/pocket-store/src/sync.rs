//! NET-22: chat sync as a replicated op log.
//!
//! Every conversation/message change is an [`Op`]; the `conversations` and
//! `messages` tables are the materialized view of the ops. Local writes
//! (through `ConversationsRepo`/`MessagesRepo`) and remote ops go through the
//! same [`apply`], which is idempotent and order-independent, so two replicas
//! that hold the same set of ops hold the same chats:
//! - conversation metadata (title, model, archived, ...) is last-writer-wins
//!   on `(lamport, device_id)`; the create op is the baseline;
//! - messages are append-only, ordered by `(created_at, lamport)`; a
//!   regenerate is a sibling of the message it replaces (`sibling_of`), the
//!   newest sibling is the visible one, older ones are kept;
//! - deletes are tombstones that beat every other op on that entity, kept
//!   [`TOMBSTONE_TTL_MS`].
//!
//! Transport: each replica keeps a version vector (`sync_state`: highest seq
//! held per origin device). [`ops_since`] returns what a peer lacks in
//! `(lamport, device_id)` order, which is also per-device seq order, so any
//! page is a per-device prefix and the vector can advance per page - an
//! interrupted sync resumes where it stopped.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::Store;
use crate::repos::conversations::ConversationRow;
use crate::repos::messages::MessageRow;

pub const TOMBSTONE_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const KEY_DEVICE: &str = "sync.device_id";
const KEY_LAMPORT: &str = "sync.lamport";
/// Conversation columns a field op may set.
const CONV_FIELDS: &[&str] = &[
    "title",
    "project_id",
    "model_id",
    "system_prompt",
    "params_json",
    "archived",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Op {
    pub op_id: String,
    pub device_id: String,
    pub seq: i64,
    pub lamport: i64,
    /// "conv" | "msg"
    pub entity: String,
    pub entity_id: String,
    pub conv_id: String,
    /// "create" | "delete" | a conversation column from `CONV_FIELDS`.
    pub field: String,
    pub value: Value,
    pub ts: i64,
}

pub type VersionVector = BTreeMap<String, i64>;

// ---- local writes (called by the repos) ----

pub(crate) fn emit(
    store: &Store,
    entity: &str,
    entity_id: &str,
    conv_id: &str,
    field: &str,
    value: Value,
    ts: i64,
) -> rusqlite::Result<()> {
    let mut conn = store.writer();
    let tx = conn.transaction()?;
    let op = new_local_op(&tx, None, entity, entity_id, conv_id, field, value, ts)?;
    apply(&tx, &op, true, store.blob_dir())?;
    tx.commit()
}

pub(crate) fn emit_message(
    store: &Store,
    row: &MessageRow,
    sibling_of: Option<&str>,
) -> rusqlite::Result<()> {
    let mut conn = store.writer();
    let tx = conn.transaction()?;
    // Siblings group under the root message so regenerating twice keeps one group.
    let root = match sibling_of {
        Some(s) => Some(
            tx.query_row(
                "SELECT COALESCE(sibling_of, id) FROM messages WHERE id = ?1",
                [s],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_else(|| s.to_string()),
        ),
        None => None,
    };
    let blobs = register_local_blobs(&tx, &row.content_json)?;
    let value = message_value(row, root.as_deref(), blobs);
    let op = new_local_op(
        &tx,
        None,
        "msg",
        &row.id,
        &row.conversation_id,
        "create",
        value,
        row.created_at,
    )?;
    apply(&tx, &op, true, store.blob_dir())?;
    tx.commit()
}

pub(crate) fn conversation_value(row: &ConversationRow) -> Value {
    json!({
        "title": row.title, "project_id": row.project_id, "model_id": row.model_id,
        "system_prompt": row.system_prompt, "params_json": row.params_json,
        "created_at": row.created_at, "updated_at": row.updated_at, "archived": row.archived,
    })
}

fn message_value(row: &MessageRow, sibling_of: Option<&str>, blobs: Map<String, Value>) -> Value {
    json!({
        "role": row.role, "content_json": row.content_json, "reasoning": row.reasoning,
        "stats_json": row.stats_json, "model_id": row.model_id, "created_at": row.created_at,
        "status": row.status, "sibling_of": sibling_of, "blobs": blobs,
    })
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---- replica identity + clocks ----

fn setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// This replica's stable sync identity (generated once, never the pairing id).
pub fn device_id(store: &Store) -> rusqlite::Result<String> {
    device_id_in(&store.writer())
}

fn device_id_in(conn: &Connection) -> rusqlite::Result<String> {
    if let Some(id) = setting(conn, KEY_DEVICE)? {
        return Ok(id);
    }
    let id = pocket_types::new_id();
    set_setting(conn, KEY_DEVICE, &id)?;
    Ok(id)
}

fn lamport(conn: &Connection) -> rusqlite::Result<i64> {
    Ok(setting(conn, KEY_LAMPORT)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

fn vv_of(conn: &Connection, device: &str) -> rusqlite::Result<i64> {
    Ok(conn
        .query_row(
            "SELECT max_seq FROM sync_state WHERE device_id = ?1",
            [device],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

fn bump_vv(conn: &Connection, device: &str, seq: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sync_state (device_id, max_seq) VALUES (?1, ?2)
         ON CONFLICT(device_id) DO UPDATE SET max_seq = MAX(max_seq, excluded.max_seq)",
        params![device, seq],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn new_local_op(
    conn: &Connection,
    op_id: Option<String>,
    entity: &str,
    entity_id: &str,
    conv_id: &str,
    field: &str,
    value: Value,
    ts: i64,
) -> rusqlite::Result<Op> {
    let device_id = device_id_in(conn)?;
    let seq = vv_of(conn, &device_id)? + 1;
    let lamport = lamport(conn)? + 1;
    let op = Op {
        op_id: op_id.unwrap_or_else(pocket_types::new_id),
        device_id,
        seq,
        lamport,
        entity: entity.into(),
        entity_id: entity_id.into(),
        conv_id: conv_id.into(),
        field: field.into(),
        value,
        ts,
    };
    bump_vv(conn, &op.device_id, seq)?;
    set_setting(conn, KEY_LAMPORT, &lamport.to_string())?;
    insert_op(conn, &op)?;
    Ok(op)
}

fn insert_op(conn: &Connection, op: &Op) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO sync_ops (op_id, device_id, seq, lamport, entity, entity_id, conv_id, field, value, ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![op.op_id, op.device_id, op.seq, op.lamport, op.entity, op.entity_id, op.conv_id,
            op.field, op.value.to_string(), op.ts],
    )?;
    Ok(n == 1)
}

fn map_op(r: &rusqlite::Row) -> rusqlite::Result<Op> {
    let value: String = r.get(8)?;
    Ok(Op {
        op_id: r.get(0)?,
        device_id: r.get(1)?,
        seq: r.get(2)?,
        lamport: r.get(3)?,
        entity: r.get(4)?,
        entity_id: r.get(5)?,
        conv_id: r.get(6)?,
        field: r.get(7)?,
        value: serde_json::from_str(&value).unwrap_or(Value::Null),
        ts: r.get(9)?,
    })
}

const OP_COLS: &str =
    "op_id, device_id, seq, lamport, entity, entity_id, conv_id, field, value, ts";

// ---- materialization ----

fn apply(conn: &Connection, op: &Op, local: bool, blob_dir: Option<&Path>) -> rusqlite::Result<()> {
    match (op.entity.as_str(), op.field.as_str()) {
        ("conv", "delete") => {
            conn.execute("DELETE FROM conversations WHERE id = ?1", [&op.entity_id])?;
        }
        ("conv", _) => materialize_conv(conn, &op.entity_id, blob_dir)?,
        ("msg", "create") => insert_message(conn, op, local, blob_dir)?,
        ("msg", "delete") => {
            conn.execute("DELETE FROM messages WHERE id = ?1", [&op.entity_id])?;
        }
        // An op kind from a newer peer: kept in the log, ignored here.
        _ => {}
    }
    Ok(())
}

fn tombstoned(conn: &Connection, entity_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_ops WHERE entity_id = ?1 AND field = 'delete')",
        [entity_id],
        |r| r.get(0),
    )
}

/// Rebuilds one conversation row from its ops (create baseline, then field
/// ops in `(lamport, device_id)` order). A row with no create op yet (an
/// unseeded pre-sync chat) uses the current row as the baseline.
fn materialize_conv(conn: &Connection, id: &str, blob_dir: Option<&Path>) -> rusqlite::Result<()> {
    if tombstoned(conn, id)? {
        conn.execute("DELETE FROM conversations WHERE id = ?1", [id])?;
        return Ok(());
    }
    let ops: Vec<(String, String, String, i64)> = conn
        .prepare(
            "SELECT op_id, field, value, ts FROM sync_ops WHERE entity = 'conv' AND entity_id = ?1
             ORDER BY lamport, device_id, op_id",
        )?
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let existing = conn
        .query_row(
            "SELECT title, project_id, model_id, system_prompt, params_json, created_at, updated_at, archived
             FROM conversations WHERE id = ?1",
            [id],
            |r| {
                Ok(conversation_value(&ConversationRow {
                    id: id.into(),
                    title: r.get(0)?,
                    project_id: r.get(1)?,
                    model_id: r.get(2)?,
                    system_prompt: r.get(3)?,
                    params_json: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                    archived: r.get::<_, i64>(7)? != 0,
                }))
            },
        )
        .optional()?;
    // Two creates for one id only come from seeding on both sides; pick one
    // deterministically so both replicas use the same baseline.
    let create = ops
        .iter()
        .filter(|o| o.1 == "create")
        .min_by(|a, b| a.0.cmp(&b.0))
        .and_then(|o| serde_json::from_str::<Value>(&o.2).ok());
    let Some(Value::Object(mut state)) = create.or_else(|| existing.clone()) else {
        return Ok(());
    };
    let mut updated_at = state.get("updated_at").and_then(Value::as_i64).unwrap_or(0);
    for (_, field, value, ts) in ops.iter().filter(|o| CONV_FIELDS.contains(&o.1.as_str())) {
        state.insert(
            field.clone(),
            serde_json::from_str(value).unwrap_or(Value::Null),
        );
        if field == "title" || field == "model_id" {
            updated_at = updated_at.max(*ts);
        }
    }
    let s = |k: &str| state.get(k).and_then(Value::as_str).map(str::to_string);
    conn.execute(
        "INSERT INTO conversations (id, title, project_id, model_id, system_prompt, params_json, created_at, updated_at, archived)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET title = excluded.title, project_id = excluded.project_id,
           model_id = excluded.model_id, system_prompt = excluded.system_prompt, params_json = excluded.params_json,
           created_at = excluded.created_at, updated_at = excluded.updated_at, archived = excluded.archived",
        params![id, s("title"), s("project_id"), s("model_id"), s("system_prompt"), s("params_json"),
            state.get("created_at").and_then(Value::as_i64).unwrap_or(0), updated_at,
            state.get("archived").and_then(Value::as_bool).unwrap_or(false) as i64],
    )?;
    if existing.is_none() {
        // Messages that arrived before their conversation did.
        let pending: Vec<Op> = conn
            .prepare(&format!(
                "SELECT {OP_COLS} FROM sync_ops WHERE conv_id = ?1 AND entity = 'msg' AND field = 'create'"
            ))?
            .query_map([id], map_op)?
            .collect::<rusqlite::Result<_>>()?;
        for op in &pending {
            insert_message(conn, op, false, blob_dir)?;
        }
    }
    Ok(())
}

fn insert_message(
    conn: &Connection,
    op: &Op,
    local: bool,
    blob_dir: Option<&Path>,
) -> rusqlite::Result<()> {
    if tombstoned(conn, &op.conv_id)? || tombstoned(conn, &op.entity_id)? {
        return Ok(());
    }
    // Remote: the conversation may not have arrived yet; materialize_conv
    // replays this op when it does. Local: let the FK error surface as before.
    let conv_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?1)",
        [&op.conv_id],
        |r| r.get(0),
    )?;
    if !local && !conv_exists {
        return Ok(());
    }
    let v = &op.value;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let content = rewrite_blob_paths(
        conn,
        &s("content_json").unwrap_or_else(|| "{}".into()),
        &v["blobs"],
        blob_dir,
    )?;
    let verb = if local { "INSERT" } else { "INSERT OR IGNORE" };
    conn.execute(
        &format!(
            "{verb} INTO messages (id, conversation_id, role, content_json, reasoning, stats_json, model_id, created_at, status, lamport, sibling_of)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
        ),
        params![op.entity_id, op.conv_id, s("role").unwrap_or_default(), content, s("reasoning"),
            s("stats_json"), s("model_id"), v.get("created_at").and_then(Value::as_i64).unwrap_or(op.ts),
            s("status").unwrap_or_else(|| "done".into()), op.lamport, s("sibling_of")],
    )?;
    Ok(())
}

// ---- attachments (content-hashed blobs) ----

fn is_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn image_paths(content_json: &str) -> Vec<String> {
    serde_json::from_str::<Value>(content_json)
        .ok()
        .and_then(|v| v.get("parts").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .filter(|p| p.get("type").and_then(Value::as_str) == Some("imageRef"))
        .filter_map(|p| p.get("path").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// Hashes this message's image files into `sync_blobs`; returns path -> hash.
fn register_local_blobs(
    conn: &Connection,
    content_json: &str,
) -> rusqlite::Result<Map<String, Value>> {
    let mut out = Map::new();
    for path in image_paths(content_json) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let hash = sha256_hex(&bytes);
        conn.execute(
            "INSERT INTO sync_blobs (sha256, path) VALUES (?1, ?2)
             ON CONFLICT(sha256) DO UPDATE SET path = COALESCE(sync_blobs.path, excluded.path)",
            params![hash, path],
        )?;
        out.insert(path, Value::String(hash));
    }
    Ok(out)
}

/// Points a message's image parts at this device's copy of each blob (or
/// where it will land once fetched), marking unfetched blobs missing.
fn rewrite_blob_paths(
    conn: &Connection,
    content_json: &str,
    blobs: &Value,
    blob_dir: Option<&Path>,
) -> rusqlite::Result<String> {
    let Some(map) = blobs.as_object().filter(|m| !m.is_empty()) else {
        return Ok(content_json.to_string());
    };
    let Ok(mut v) = serde_json::from_str::<Value>(content_json) else {
        return Ok(content_json.to_string());
    };
    if let Some(parts) = v.get_mut("parts").and_then(Value::as_array_mut) {
        for part in parts {
            let hash = part
                .get("path")
                .and_then(Value::as_str)
                .and_then(|p| map.get(p))
                .and_then(Value::as_str)
                .filter(|h| is_hash(h))
                .map(str::to_string);
            let Some(hash) = hash else { continue };
            let known: Option<Option<String>> = conn
                .query_row(
                    "SELECT path FROM sync_blobs WHERE sha256 = ?1",
                    [&hash],
                    |r| r.get(0),
                )
                .optional()?;
            let local = match known {
                Some(Some(p)) => Some(p),
                _ => {
                    conn.execute(
                        "INSERT OR IGNORE INTO sync_blobs (sha256, path) VALUES (?1, NULL)",
                        [&hash],
                    )?;
                    blob_dir.map(|d| d.join(&hash).to_string_lossy().into_owned())
                }
            };
            if let Some(local) = local {
                part["path"] = Value::String(local);
            }
        }
    }
    Ok(v.to_string())
}

/// Blobs referenced by synced messages that this device doesn't have yet.
pub fn missing_blobs(store: &Store) -> rusqlite::Result<Vec<String>> {
    let conn = store.reader();
    let mut stmt = conn.prepare("SELECT sha256 FROM sync_blobs WHERE path IS NULL")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

pub fn blob_path(store: &Store, hash: &str) -> Option<PathBuf> {
    if !is_hash(hash) {
        return None;
    }
    let path: String = store
        .reader()
        .query_row(
            "SELECT path FROM sync_blobs WHERE sha256 = ?1 AND path IS NOT NULL",
            [hash],
            |r| r.get(0),
        )
        .ok()?;
    let path = PathBuf::from(path);
    path.is_file().then_some(path)
}

/// Stores a fetched blob. Only accepts blobs a synced message references and
/// whose bytes match the hash, so a peer can't write arbitrary files here.
pub fn put_blob(store: &Store, hash: &str, bytes: &[u8]) -> rusqlite::Result<bool> {
    let Some(dir) = store.blob_dir() else {
        return Ok(false);
    };
    if !is_hash(hash)
        || sha256_hex(bytes) != hash
        || !missing_blobs(store)?.iter().any(|h| h == hash)
    {
        return Ok(false);
    }
    let path = dir.join(hash);
    std::fs::create_dir_all(dir)
        .and_then(|_| std::fs::write(&path, bytes))
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    store.writer().execute(
        "UPDATE sync_blobs SET path = ?2 WHERE sha256 = ?1",
        params![hash, path.to_string_lossy()],
    )?;
    Ok(true)
}

// ---- replication ----

/// Records ops for chats that predate the op log (or were written around
/// it). Seed op ids are content hashes, so the same chat seeded on both
/// devices yields the same op and is not duplicated. Cheap when nothing is
/// unseeded, so it runs at every sync.
pub fn seed(store: &Store) -> rusqlite::Result<usize> {
    let mut conn = store.writer();
    let tx = conn.transaction()?;
    let convs: Vec<ConversationRow> = tx
        .prepare(
            "SELECT id, title, project_id, model_id, system_prompt, params_json, created_at, updated_at, archived
             FROM conversations c WHERE NOT EXISTS
               (SELECT 1 FROM sync_ops o WHERE o.entity_id = c.id AND o.field = 'create')",
        )?
        .query_map([], |r| {
            Ok(ConversationRow {
                id: r.get(0)?,
                title: r.get(1)?,
                project_id: r.get(2)?,
                model_id: r.get(3)?,
                system_prompt: r.get(4)?,
                params_json: r.get(5)?,
                created_at: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                updated_at: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                archived: r.get::<_, Option<i64>>(8)?.unwrap_or(0) != 0,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    for c in &convs {
        let value = conversation_value(c);
        let id = seed_op_id("conv", &c.id, &value);
        new_local_op(
            &tx,
            Some(id),
            "conv",
            &c.id,
            &c.id,
            "create",
            value,
            c.created_at,
        )?;
    }
    let msgs: Vec<(MessageRow, Option<String>)> = tx
        .prepare(
            "SELECT id, conversation_id, role, content_json, reasoning, stats_json, model_id, created_at, status, sibling_of
             FROM messages m WHERE NOT EXISTS
               (SELECT 1 FROM sync_ops o WHERE o.entity_id = m.id AND o.field = 'create')
             ORDER BY created_at, id",
        )?
        .query_map([], |r| {
            Ok((
                MessageRow {
                    id: r.get(0)?,
                    conversation_id: r.get(1)?,
                    role: r.get(2)?,
                    content_json: r.get(3)?,
                    reasoning: r.get(4)?,
                    stats_json: r.get(5)?,
                    model_id: r.get(6)?,
                    created_at: r.get(7)?,
                    status: r.get(8)?,
                },
                r.get(9)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (m, sibling_of) in &msgs {
        let blobs = register_local_blobs(&tx, &m.content_json)?;
        let value = message_value(m, sibling_of.as_deref(), blobs);
        let id = seed_op_id("msg", &m.id, &value);
        let op = new_local_op(
            &tx,
            Some(id),
            "msg",
            &m.id,
            &m.conversation_id,
            "create",
            value,
            m.created_at,
        )?;
        tx.execute(
            "UPDATE messages SET lamport = ?2 WHERE id = ?1",
            params![m.id, op.lamport],
        )?;
    }
    tx.commit()?;
    Ok(convs.len() + msgs.len())
}

fn seed_op_id(entity: &str, entity_id: &str, value: &Value) -> String {
    let hash = sha256_hex(format!("{entity}\0{entity_id}\0{value}").as_bytes());
    format!("seed-{}", &hash[..32])
}

pub fn version_vector(store: &Store) -> rusqlite::Result<VersionVector> {
    let conn = store.reader();
    let mut stmt = conn.prepare("SELECT device_id, max_seq FROM sync_state")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// Up to `limit` ops the holder of `vv` lacks, in causal order, and whether
/// more remain.
pub fn ops_since(
    store: &Store,
    vv: &VersionVector,
    limit: usize,
) -> rusqlite::Result<(Vec<Op>, bool)> {
    let conn = store.reader();
    let vv_json = serde_json::to_string(vv).unwrap_or_else(|_| "{}".into());
    let mut stmt = conn.prepare(&format!(
        "SELECT {OP_COLS} FROM sync_ops o
         WHERE o.seq > COALESCE((SELECT j.value FROM json_each(?1) j WHERE j.key = o.device_id), 0)
         ORDER BY o.lamport, o.device_id LIMIT ?2"
    ))?;
    let mut ops: Vec<Op> = stmt
        .query_map(params![vv_json, limit as i64 + 1], map_op)?
        .collect::<rusqlite::Result<_>>()?;
    let more = ops.len() > limit;
    ops.truncate(limit);
    Ok((ops, more))
}

/// Applies a peer's ops (idempotent). Returns how many were new.
pub fn apply_remote(store: &Store, ops: &[Op]) -> rusqlite::Result<usize> {
    let mut conn = store.writer();
    let tx = conn.transaction()?;
    let mut new = 0;
    for op in ops {
        bump_vv(&tx, &op.device_id, op.seq)?;
        if op.lamport > lamport(&tx)? {
            set_setting(&tx, KEY_LAMPORT, &op.lamport.to_string())?;
        }
        let valid = matches!(op.entity.as_str(), "conv" | "msg")
            && (op.entity != "conv" || op.conv_id == op.entity_id);
        if valid && insert_op(&tx, op)? {
            apply(&tx, op, false, store.blob_dir())?;
            new += 1;
        }
    }
    tx.commit()?;
    Ok(new)
}

/// Conversations with this device's ops that a peer holding `peer_has` of
/// our seq hasn't received yet (the outbox, by chat).
pub fn pending_conversations(store: &Store, peer_has: i64) -> rusqlite::Result<(u64, Vec<String>)> {
    let me = device_id(store)?;
    let conn = store.reader();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sync_ops WHERE device_id = ?1 AND seq > ?2",
        params![me, peer_has],
        |r| r.get(0),
    )?;
    let mut stmt =
        conn.prepare("SELECT DISTINCT conv_id FROM sync_ops WHERE device_id = ?1 AND seq > ?2")?;
    let ids = stmt
        .query_map(params![me, peer_has], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok((count as u64, ids))
}

/// Drops every op of an entity whose tombstone is older than 30 days.
/// ponytail: a device offline longer than that can resurrect such a chat on
/// reconnect; keep a bare tombstone id list if that matters in practice.
pub fn gc(store: &Store, now: i64) -> rusqlite::Result<()> {
    let cutoff = now - TOMBSTONE_TTL_MS;
    let conn = store.writer();
    conn.execute(
        "DELETE FROM sync_ops WHERE conv_id IN
           (SELECT entity_id FROM sync_ops WHERE entity = 'conv' AND field = 'delete' AND ts < ?1)",
        [cutoff],
    )?;
    conn.execute(
        "DELETE FROM sync_ops WHERE entity = 'msg' AND entity_id IN
           (SELECT entity_id FROM sync_ops WHERE entity = 'msg' AND field = 'delete' AND ts < ?1)",
        [cutoff],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConversationsRepo, MessagesRepo};

    fn conv(id: &str, t: i64) -> ConversationRow {
        ConversationRow {
            id: id.into(),
            title: Some(format!("chat {id}")),
            project_id: None,
            model_id: None,
            system_prompt: None,
            params_json: None,
            created_at: t,
            updated_at: t,
            archived: false,
        }
    }

    fn msg(id: &str, conv: &str, t: i64, text: &str) -> MessageRow {
        MessageRow {
            id: id.into(),
            conversation_id: conv.into(),
            role: "user".into(),
            content_json: json!({ "text": text }).to_string(),
            reasoning: None,
            stats_json: None,
            model_id: None,
            created_at: t,
            status: "done".into(),
        }
    }

    /// One direction, `pages` pages of `page` ops (an interrupted sync when
    /// that's not enough). Returns whether `to` is caught up.
    fn pull(from: &Store, to: &Store, page: usize, pages: usize) -> bool {
        for _ in 0..pages {
            let (ops, more) = ops_since(from, &version_vector(to).unwrap(), page).unwrap();
            apply_remote(to, &ops).unwrap();
            if !more {
                return true;
            }
        }
        false
    }

    fn full_sync(a: &Store, b: &Store) {
        seed(a).unwrap();
        seed(b).unwrap();
        while !pull(a, b, 7, 1000) {}
        while !pull(b, a, 7, 1000) {}
    }

    /// Everything the user can see, in display order.
    fn snapshot(store: &Store) -> Vec<String> {
        let mut out = Vec::new();
        for c in ConversationsRepo::new(store).list().unwrap() {
            out.push(format!("{c:?}"));
            for m in MessagesRepo::new(store)
                .list_for_conversation(&c.id)
                .unwrap()
            {
                out.push(format!("  {m:?}"));
            }
        }
        out.sort();
        out
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn random_edit(store: &Store, rng: &mut Rng, tag: &str, n: usize) {
        let convs = ConversationsRepo::new(store);
        let msgs = MessagesRepo::new(store);
        let ids: Vec<String> = convs.list().unwrap().into_iter().map(|c| c.id).collect();
        let pick = |rng: &mut Rng| (!ids.is_empty()).then(|| ids[rng.below(ids.len())].clone());
        let t = (n as i64) / 3; // coarse clock: plenty of created_at ties
        match rng.below(9) {
            0 | 1 => convs.create(&conv(&format!("c{tag}{n}"), t)).unwrap(),
            2 => {
                if let Some(id) = pick(rng) {
                    convs.update_title(&id, &format!("t{tag}{n}"), t).unwrap()
                }
            }
            3 => {
                if let Some(id) = pick(rng) {
                    convs.archive(&id, rng.below(2) == 0).unwrap()
                }
            }
            4 => {
                if let Some(id) = pick(rng) {
                    convs.update_model(&id, &format!("m{tag}{n}"), t).unwrap()
                }
            }
            5 => {
                if let Some(id) = pick(rng) {
                    if rng.below(4) == 0 {
                        convs.delete(&id).unwrap()
                    }
                }
            }
            6 | 7 => {
                if let Some(id) = pick(rng) {
                    msgs.create(&msg(&format!("m{tag}{n}"), &id, t, tag))
                        .unwrap()
                }
            }
            _ => {
                if let Some(id) = pick(rng) {
                    let list = msgs.list_for_conversation(&id).unwrap();
                    if let Some(last) = list.last() {
                        if rng.below(3) == 0 {
                            msgs.delete(&last.id).unwrap();
                        } else {
                            msgs.create_sibling(
                                &msg(&format!("r{tag}{n}"), &id, t, "regen"),
                                &last.id,
                            )
                            .unwrap();
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn random_interleavings_converge() {
        for seed_value in 1..=40u64 {
            let mut rng = Rng(seed_value.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let (a, b) = (
                Store::open_in_memory().unwrap(),
                Store::open_in_memory().unwrap(),
            );
            for n in 0..120 {
                match rng.below(10) {
                    0..=3 => random_edit(&a, &mut rng, "a", n),
                    4..=7 => random_edit(&b, &mut rng, "b", n),
                    // Partial, possibly interrupted, one-way syncs.
                    8 => {
                        let (p, k) = (1 + rng.below(5), 1 + rng.below(3));
                        pull(&a, &b, p, k);
                    }
                    _ => {
                        let (p, k) = (1 + rng.below(5), 1 + rng.below(3));
                        pull(&b, &a, p, k);
                    }
                }
            }
            full_sync(&a, &b);
            assert!(!snapshot(&a).is_empty());
            assert_eq!(snapshot(&a), snapshot(&b), "seed {seed_value}");
            assert_eq!(version_vector(&a).unwrap(), version_vector(&b).unwrap());
        }
    }

    #[test]
    fn delete_propagates_and_beats_concurrent_edits() {
        let (a, b) = (
            Store::open_in_memory().unwrap(),
            Store::open_in_memory().unwrap(),
        );
        ConversationsRepo::new(&a).create(&conv("c1", 1)).unwrap();
        MessagesRepo::new(&a)
            .create(&msg("m1", "c1", 1, "hi"))
            .unwrap();
        full_sync(&a, &b);
        assert_eq!(
            MessagesRepo::new(&b)
                .list_for_conversation("c1")
                .unwrap()
                .len(),
            1
        );
        // Offline: A deletes, B renames and adds a message.
        ConversationsRepo::new(&a).delete("c1").unwrap();
        ConversationsRepo::new(&b)
            .update_title("c1", "renamed", 2)
            .unwrap();
        MessagesRepo::new(&b)
            .create(&msg("m2", "c1", 2, "late"))
            .unwrap();
        full_sync(&a, &b);
        assert!(ConversationsRepo::new(&a).get("c1").unwrap().is_none());
        assert!(ConversationsRepo::new(&b).get("c1").unwrap().is_none());
        assert!(MessagesRepo::new(&b).get("m2").unwrap().is_none());
        // Tombstones outlive 30 days only until gc drops them with their ops.
        let now = now_ms();
        gc(&a, now).unwrap();
        assert!(tombstoned(&a.writer(), "c1").unwrap());
        gc(&a, now + TOMBSTONE_TTL_MS + 1).unwrap();
        let left: i64 = a
            .writer()
            .query_row(
                "SELECT COUNT(*) FROM sync_ops WHERE conv_id = 'c1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn regenerate_keeps_both_siblings_and_shows_the_newest() {
        let (a, b) = (
            Store::open_in_memory().unwrap(),
            Store::open_in_memory().unwrap(),
        );
        ConversationsRepo::new(&a).create(&conv("c1", 1)).unwrap();
        MessagesRepo::new(&a)
            .create(&msg("r1", "c1", 1, "first"))
            .unwrap();
        full_sync(&a, &b);
        // Both devices regenerate the same reply while apart.
        MessagesRepo::new(&a)
            .create_sibling(&msg("r2", "c1", 2, "a"), "r1")
            .unwrap();
        MessagesRepo::new(&b)
            .create_sibling(&msg("r3", "c1", 3, "b"), "r1")
            .unwrap();
        full_sync(&a, &b);
        for s in [&a, &b] {
            let visible = MessagesRepo::new(s).list_for_conversation("c1").unwrap();
            assert_eq!(
                visible.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
                ["r3"]
            );
            for id in ["r1", "r2", "r3"] {
                assert!(MessagesRepo::new(s).get(id).unwrap().is_some(), "{id} kept");
            }
        }
    }

    #[test]
    fn seeding_identical_legacy_chats_on_both_sides_does_not_duplicate() {
        let (a, b) = (
            Store::open_in_memory().unwrap(),
            Store::open_in_memory().unwrap(),
        );
        for s in [&a, &b] {
            // Pre-NET-22 rows: written straight to the tables, no ops.
            s.writer().execute_batch(
                "INSERT INTO conversations (id, title, created_at, updated_at) VALUES ('c1', 'same', 1, 1);
                 INSERT INTO messages (id, conversation_id, role, content_json, created_at, status)
                   VALUES ('m1', 'c1', 'user', '{\"text\":\"hi\"}', 1, 'done');",
            ).unwrap();
        }
        full_sync(&a, &b);
        full_sync(&a, &b);
        for s in [&a, &b] {
            let ops: i64 = s
                .writer()
                .query_row("SELECT COUNT(*) FROM sync_ops", [], |r| r.get(0))
                .unwrap();
            assert_eq!(ops, 2, "one conv + one message op, not one per device");
            assert_eq!(
                MessagesRepo::new(s)
                    .list_for_conversation("c1")
                    .unwrap()
                    .len(),
                1
            );
        }
        assert_eq!(snapshot(&a), snapshot(&b));
    }

    #[test]
    fn image_blobs_are_hashed_rewritten_and_only_referenced_ones_accepted() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = Store::open(dir_a.path().join("a.sqlite")).unwrap();
        let b = Store::open(dir_b.path().join("b.sqlite")).unwrap();
        let img = dir_a.path().join("photo.jpg");
        std::fs::write(&img, b"jpeg bytes").unwrap();
        ConversationsRepo::new(&a).create(&conv("c1", 1)).unwrap();
        let mut m = msg("m1", "c1", 1, "look");
        m.content_json = json!({ "text": "look", "parts": [{ "type": "imageRef", "path": img.to_string_lossy() }] }).to_string();
        MessagesRepo::new(&a).create(&m).unwrap();
        full_sync(&a, &b);

        let hash = sha256_hex(b"jpeg bytes");
        assert_eq!(missing_blobs(&b).unwrap(), vec![hash.clone()]);
        let got = MessagesRepo::new(&b).get("m1").unwrap().unwrap();
        let expected = b.blob_dir().unwrap().join(&hash);
        assert!(got.content_json.contains(&*expected.to_string_lossy()));
        // Wrong bytes and unreferenced hashes are refused.
        assert!(!put_blob(&b, &hash, b"evil").unwrap());
        assert!(!put_blob(&b, &sha256_hex(b"x"), b"x").unwrap());
        let bytes = std::fs::read(blob_path(&a, &hash).unwrap()).unwrap();
        assert!(put_blob(&b, &hash, &bytes).unwrap());
        assert!(missing_blobs(&b).unwrap().is_empty());
        assert_eq!(std::fs::read(expected).unwrap(), b"jpeg bytes");
    }
}
