use crate::error::{Code, CommsError, Result};
use crate::policy::{KINDS, STATUSES};
use rusqlite::{Connection, OptionalExtension, Row};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

fn sql_in(values: &[&str]) -> String {
    format!("({})", values.iter().map(|value| format!("'{value}'")).collect::<Vec<_>>().join(", "))
}

pub fn schema_sql() -> String {
    format!(r#"
CREATE TABLE IF NOT EXISTS turns (
  id INTEGER PRIMARY KEY,
  sender TEXT NOT NULL,
  recipient TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN {}),
  title TEXT NOT NULL,
  body TEXT NOT NULL,
  refs TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  resolved_at TEXT
);
CREATE TABLE IF NOT EXISTS work_items (
  id INTEGER PRIMARY KEY,
  title TEXT NOT NULL,
  owner TEXT NOT NULL,
  scope TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'aberto' CHECK (status IN {}),
  blocker TEXT,
  refs TEXT,
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS kv (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_turns_recipient ON turns(recipient);
CREATE INDEX IF NOT EXISTS idx_turns_sender ON turns(sender);
CREATE INDEX IF NOT EXISTS idx_turns_resolved ON turns(resolved_at);
CREATE INDEX IF NOT EXISTS idx_work_items_owner ON work_items(owner);
CREATE INDEX IF NOT EXISTS idx_work_items_status ON work_items(status);
CREATE TABLE IF NOT EXISTS agents (
  name TEXT PRIMARY KEY,
  description TEXT NOT NULL,
  prompt TEXT NOT NULL,
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
"#, sql_in(&KINDS), sql_in(&STATUSES))
}

pub fn open_rw(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(30))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=100;")?;
    conn.execute_batch(&schema_sql())?;
    Ok(conn)
}

pub fn open_ro(path: &Path) -> Result<Connection> {
    if !path.exists() {
        return Err(CommsError::new(
            Code::DbMissing,
            format!("ERRO: Banco de dados '{}' não existe. Por favor, execute 'comms bootstrap' primeiro.", path.display()),
        ));
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(30))?;
    Ok(conn)
}

pub fn checkpoint(conn: &Connection) {
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
}

pub fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )?)
}

pub fn kv_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT value FROM kv WHERE key=?1", [key], |row| row.get(0)).optional()?)
}

#[derive(Debug, Serialize)]
pub struct Turn {
    pub id: i64,
    pub sender: String,
    pub recipient: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub refs: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

impl Turn {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?, sender: row.get("sender")?, recipient: row.get("recipient")?,
            kind: row.get("kind")?, title: row.get("title")?, body: row.get("body")?,
            refs: row.get("refs")?, created_at: row.get("created_at")?, resolved_at: row.get("resolved_at")?,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct WorkItem {
    pub id: i64,
    pub title: String,
    pub owner: String,
    pub scope: String,
    pub status: String,
    pub blocker: Option<String>,
    pub refs: Option<String>,
    pub updated_at: String,
}

impl WorkItem {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?, title: row.get("title")?, owner: row.get("owner")?,
            scope: row.get("scope")?, status: row.get("status")?, blocker: row.get("blocker")?,
            refs: row.get("refs")?, updated_at: row.get("updated_at")?,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct Kv {
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

impl Kv {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self { key: row.get("key")?, value: row.get("value")?, updated_at: row.get("updated_at")? })
    }
}

#[derive(Debug, Serialize)]
pub struct AgentRow {
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

pub fn turn_by_id(conn: &Connection, id: i64) -> Result<Option<Turn>> {
    Ok(conn.query_row("SELECT * FROM turns WHERE id=?1", [id], Turn::from_row).optional()?)
}

pub fn item_by_id(conn: &Connection, id: i64) -> Result<Option<WorkItem>> {
    Ok(conn.query_row("SELECT * FROM work_items WHERE id=?1", [id], WorkItem::from_row).optional()?)
}
