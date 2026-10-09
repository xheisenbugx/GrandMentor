//! Whole-profile export / import (backup file) and device sync.
//!
//! The backup is one JSON document:
//!
//! ```json
//! { "format": "grandmentor-backup", "format_version": 1, "schema_version": 2,
//!   "app_version": "0.1.0", "created_at": "2026-10-08T12:00:00Z",
//!   "profile": { ...Profile },
//!   "tables": { "games": [ { "id": 1, "white": "...", ... } ], "activity": [ ... ] },
//!   "browser": { "grandmentor.settings.v1": "{...}" } }   // added by the frontend
//! ```
//!
//! * **Generic**: every user table is discovered from `pragma_table_list` (SQLite internals and
//!   [`EXCLUDED`] tables are skipped), so tables added by future features are included
//!   automatically. Rows are JSON objects keyed by column name.
//! * **Import** validates the format version, skips unknown tables / columns with a warning and
//!   runs inside one transaction, in one of two modes:
//!   * [`ImportMode::Replace`] — wipe every user table and restore the backup verbatim.
//!   * [`ImportMode::Merge`] — keep local data and add what is missing. Tables keyed by an
//!     auto-increment `INTEGER PRIMARY KEY` get fresh ids (rows referencing them through a
//!     foreign key, or a `game_id` column, are remapped); rows are de-duplicated by content
//!     (games by start position + moves + creation time). Tables with a natural primary key are
//!     upserted (newer `updated_at` wins, otherwise the local row is kept). `profile` is merged
//!     field by field. Merging the same file twice changes nothing the second time.
//! * Everything is bounded by [`MAX_BACKUP_BYTES`].
//!
//! Tables are created by [`schema`], which runs inside schema migration v2 (see `lib.rs`).

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Context;
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::Store;

/// Value of the `format` field.
pub const FORMAT: &str = "grandmentor-backup";
/// Current backup format version. Files with a newer version are rejected.
pub const FORMAT_VERSION: u64 = 1;
/// Largest backup we produce or accept.
pub const MAX_BACKUP_BYTES: usize = 200 * 1024 * 1024;
/// Largest `browser` settings object we keep.
pub const MAX_BROWSER_BYTES: usize = 512 * 1024;
/// Bookkeeping table of this module (never exported or restored).
const META_TABLE: &str = "backup_meta";
/// Tables that never travel in a backup.
pub const EXCLUDED: &[&str] = &[META_TABLE];

/// Creates this module's tables. Must be idempotent (`CREATE TABLE IF NOT EXISTS ...`).
pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS backup_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;
"#,
    )
}

// ---------------------------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------------------------

/// Why a backup file was rejected. Carried inside `anyhow::Error` so callers can localize it.
#[derive(Debug, Clone, PartialEq)]
pub enum BackupError {
    /// Not valid JSON / wrong shape.
    Malformed(String),
    /// Valid JSON, but not a GrandMentor backup.
    NotABackup,
    /// Made by a newer GrandMentor with a format this build cannot read.
    UnsupportedVersion { found: u64, supported: u64 },
    /// Bigger than [`MAX_BACKUP_BYTES`].
    TooLarge,
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::Malformed(e) => write!(f, "this file is not a valid backup: {e}"),
            BackupError::NotABackup => f.write_str("this file is not a GrandMentor backup"),
            BackupError::UnsupportedVersion { found, supported } => write!(
                f,
                "this backup uses format version {found}, but this GrandMentor only understands up to {supported}"
            ),
            BackupError::TooLarge => write!(f, "backup too large (max {} MB)", MAX_BACKUP_BYTES / (1024 * 1024)),
        }
    }
}

impl std::error::Error for BackupError {}

/// A parsed backup file.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct BackupFile {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub format_version: u64,
    #[serde(default)]
    pub schema_version: i64,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub profile: Value,
    #[serde(default)]
    pub browser: Value,
    #[serde(default)]
    pub tables: BTreeMap<String, Vec<Map<String, Value>>>,
}

/// How [`Store::import_backup`] combines the backup with local data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportMode {
    Merge,
    Replace,
}

impl ImportMode {
    pub fn parse(s: &str) -> Option<ImportMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "merge" => Some(ImportMode::Merge),
            "replace" => Some(ImportMode::Replace),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            ImportMode::Merge => "merge",
            ImportMode::Replace => "replace",
        }
    }
}

/// Which timestamp an import updates in [`BackupStatus`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportSource {
    /// A backup file restored by the user.
    File,
    /// A device sync.
    Sync,
}

/// A non-fatal problem found while previewing or importing.
/// `code`: `unknown_table` | `unknown_column` | `rows_failed` | `newer_schema` | `browser_dropped`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct BackupWarning {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    pub count: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TableCount {
    pub name: String,
    pub rows: u64,
}

/// `GET /api/backup/status`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct BackupStatus {
    pub last_backup_at: Option<String>,
    pub last_restore_at: Option<String>,
    pub last_sync_at: Option<String>,
    pub tables: Vec<TableCount>,
    pub total_rows: u64,
    pub schema_version: i64,
    pub format_version: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PreviewTable {
    pub name: String,
    /// Rows in the backup.
    pub rows: u64,
    /// Whether this database has the table (unknown tables are skipped).
    pub known: bool,
    /// Rows currently stored locally (None for unknown tables).
    pub current_rows: Option<u64>,
}

/// `POST /api/backup/preview`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct BackupPreview {
    pub format_version: u64,
    pub schema_version: i64,
    pub current_schema_version: i64,
    pub app_version: String,
    pub created_at: String,
    pub profile_name: Option<String>,
    pub tables: Vec<PreviewTable>,
    pub total_rows: u64,
    pub warnings: Vec<BackupWarning>,
    /// Sanitized browser settings carried by the file (`{}` when none).
    pub browser: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TableReport {
    pub name: String,
    pub inserted: u64,
    pub updated: u64,
    pub skipped: u64,
    pub failed: u64,
}

/// Result of [`Store::import_backup`].
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ImportReport {
    pub mode: String,
    pub tables: Vec<TableReport>,
    pub inserted: u64,
    pub updated: u64,
    pub skipped: u64,
    pub failed: u64,
    pub warnings: Vec<BackupWarning>,
    /// Sanitized browser settings to restore on the client (`{}` when none).
    pub browser: Value,
}

// ---------------------------------------------------------------------------------------------
// Parsing & helpers
// ---------------------------------------------------------------------------------------------

/// Parses and validates a backup file. Errors are [`BackupError`]s.
pub fn parse_backup(bytes: &[u8]) -> Result<BackupFile, BackupError> {
    if bytes.len() > MAX_BACKUP_BYTES {
        return Err(BackupError::TooLarge);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|e| BackupError::Malformed(short_err(&e)))?;
    let Value::Object(obj) = &v else { return Err(BackupError::NotABackup) };
    if obj.get("format").and_then(Value::as_str) != Some(FORMAT) || !obj.get("tables").is_some_and(Value::is_object) {
        return Err(BackupError::NotABackup);
    }
    let found = obj.get("format_version").and_then(Value::as_u64).unwrap_or(0);
    if found == 0 || found > FORMAT_VERSION {
        return Err(BackupError::UnsupportedVersion { found, supported: FORMAT_VERSION });
    }
    serde_json::from_value::<BackupFile>(v).map_err(|e| BackupError::Malformed(short_err(&e)))
}

fn short_err(e: &serde_json::Error) -> String {
    let s = e.to_string();
    s.chars().take(200).collect()
}

/// Keeps only `{ "grandmentor*" | "gm.*" | "gm_*" | "gm-*": "<string>" }` entries, bounded.
pub fn sanitize_browser(v: &Value) -> (Value, bool) {
    let mut out = Map::new();
    let mut dropped = false;
    let mut total = 0usize;
    if let Value::Object(map) = v {
        for (k, val) in map {
            let lk = k.to_ascii_lowercase();
            let ok_key = k.len() <= 200
                && (lk.starts_with("grandmentor") || lk.starts_with("gm.") || lk.starts_with("gm_") || lk.starts_with("gm-"));
            match val {
                Value::String(s) if ok_key && total + k.len() + s.len() <= MAX_BROWSER_BYTES => {
                    total += k.len() + s.len();
                    out.insert(k.clone(), Value::String(s.clone()));
                }
                _ => dropped = true,
            }
        }
    } else if !v.is_null() {
        dropped = true;
    }
    (Value::Object(out), dropped)
}

/// Double-quote an SQL identifier.
fn q(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn now_iso(conn: &Connection) -> rusqlite::Result<String> {
    conn.query_row(&format!("SELECT {}", crate::NOW), [], |r| r.get(0))
}

fn schema_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| s.get(i..i + 2).and_then(|p| u8::from_str_radix(p, 16).ok()))
        .collect()
}

fn sql_to_json(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => {
            let mut m = Map::new();
            m.insert("$blob".into(), Value::String(hex(b)));
            Value::Object(m)
        }
    }
}

fn json_to_sql(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => SqlValue::Integer(i),
            None => n.as_f64().map(SqlValue::Real).unwrap_or(SqlValue::Null),
        },
        Value::String(s) => SqlValue::Text(s.clone()),
        Value::Object(m) if m.len() == 1 => match m.get("$blob").and_then(Value::as_str).and_then(unhex) {
            Some(b) => SqlValue::Blob(b),
            None => SqlValue::Text(v.to_string()),
        },
        other => SqlValue::Text(other.to_string()),
    }
}

/// Stable identity of a value for de-duplication (numbers compare by value).
fn key_part(v: &SqlValue, out: &mut String) {
    use std::fmt::Write;
    match v {
        SqlValue::Null => out.push('N'),
        SqlValue::Integer(i) => {
            let _ = write!(out, "n{i}");
        }
        SqlValue::Real(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => {
            let _ = write!(out, "n{}", *f as i64);
        }
        SqlValue::Real(f) => {
            let _ = write!(out, "r{f}");
        }
        SqlValue::Text(s) => {
            let _ = write!(out, "t{}:{s}", s.len());
        }
        SqlValue::Blob(b) => {
            let _ = write!(out, "b{}", hex(b));
        }
    }
    out.push('\u{1f}');
}

// ---------------------------------------------------------------------------------------------
// Schema introspection
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct TableInfo {
    name: String,
    cols: Vec<String>,
    pk: Vec<String>,
    /// The single `INTEGER PRIMARY KEY` column of a rowid table (device-local ids).
    rowid_pk: Option<String>,
    /// (column, parent table) pairs whose values are ids of a rowid-keyed parent.
    refs: Vec<(String, String)>,
}

/// User tables (no SQLite internals, no [`EXCLUDED`]), sorted by name.
fn user_tables(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM pragma_table_list WHERE schema = 'main' AND type = 'table' \
         AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name",
    )?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names.into_iter().filter(|n| !EXCLUDED.contains(&n.as_str())).collect())
}

fn table_infos(conn: &Connection) -> anyhow::Result<Vec<TableInfo>> {
    let names = user_tables(conn)?;
    let mut infos = Vec::with_capacity(names.len());
    for name in &names {
        let without_rowid: bool = conn
            .query_row("SELECT wr FROM pragma_table_list WHERE schema = 'main' AND name = ?1", [name], |r| {
                r.get::<_, i64>(0)
            })
            .map(|v| v != 0)
            .unwrap_or(false);
        let mut stmt = conn.prepare("SELECT name, upper(type), pk FROM pragma_table_info(?1) ORDER BY cid")?;
        let cols: Vec<(String, String, i64)> = stmt
            .query_map([name], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut pk: Vec<(i64, String)> = cols.iter().filter(|c| c.2 > 0).map(|c| (c.2, c.0.clone())).collect();
        pk.sort();
        let pk: Vec<String> = pk.into_iter().map(|(_, n)| n).collect();
        let rowid_pk = match (pk.as_slice(), without_rowid) {
            ([only], false) if cols.iter().any(|c| &c.0 == only && c.1 == "INTEGER") => Some(only.clone()),
            _ => None,
        };
        infos.push(TableInfo {
            name: name.clone(),
            cols: cols.into_iter().map(|c| c.0).collect(),
            pk,
            rowid_pk,
            refs: Vec::new(),
        });
    }
    // Foreign keys pointing at rowid-keyed parents (+ undeclared `game_id` columns).
    let rowid_parents: HashMap<String, String> = infos
        .iter()
        .filter_map(|t| t.rowid_pk.clone().map(|pk| (t.name.clone(), pk)))
        .collect();
    for info in &mut infos {
        let mut stmt = conn.prepare("SELECT \"from\", \"table\", \"to\" FROM pragma_foreign_key_list(?1)")?;
        let fks: Vec<(String, String, Option<String>)> = stmt
            .query_map([&info.name], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (from, parent, to) in fks {
            if let Some(ppk) = rowid_parents.get(&parent) {
                if to.as_deref().map(|t| t == ppk).unwrap_or(true) && info.cols.contains(&from) {
                    info.refs.push((from, parent));
                }
            }
        }
        if info.name != "games"
            && rowid_parents.contains_key("games")
            && info.cols.iter().any(|c| c == "game_id")
            && !info.refs.iter().any(|(c, _)| c == "game_id")
        {
            info.refs.push(("game_id".into(), "games".into()));
        }
    }
    Ok(infos)
}

/// Parents (referenced tables) before children; cycles fall back to name order.
fn topo_order(infos: &[TableInfo]) -> Vec<usize> {
    let mut done: Vec<bool> = vec![false; infos.len()];
    let mut order = Vec::with_capacity(infos.len());
    loop {
        let mut progressed = false;
        for (i, t) in infos.iter().enumerate() {
            if done[i] {
                continue;
            }
            let ready = t.refs.iter().all(|(_, p)| {
                p == &t.name || infos.iter().position(|o| &o.name == p).map(|j| done[j]).unwrap_or(true)
            });
            if ready {
                done[i] = true;
                order.push(i);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    for (i, d) in done.iter().enumerate() {
        if !d {
            order.push(i);
        }
    }
    order
}

fn count_rows(conn: &Connection, table: &str) -> anyhow::Result<u64> {
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {}", q(table)), [], |r| r.get(0))?;
    Ok(n.max(0) as u64)
}

fn get_meta(conn: &Connection, key: &str) -> anyhow::Result<Option<String>> {
    Ok(conn
        .query_row(&format!("SELECT value FROM {META_TABLE} WHERE key = ?1"), [key], |r| r.get(0))
        .optional()?)
}

fn set_meta_now(conn: &Connection, key: &str) -> anyhow::Result<()> {
    let now = now_iso(conn)?;
    conn.execute(
        &format!("INSERT INTO {META_TABLE} (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value"),
        params![key, now],
    )?;
    Ok(())
}

fn is_constraint(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation)
}

fn warn(warnings: &mut Vec<BackupWarning>, code: &str, table: Option<&str>, column: Option<&str>, count: u64) {
    warnings.push(BackupWarning {
        code: code.into(),
        table: table.map(str::to_string),
        column: column.map(str::to_string),
        count,
    });
}

// ---------------------------------------------------------------------------------------------
// Store API
// ---------------------------------------------------------------------------------------------

impl Store {
    /// Serializes every user table into a backup document (UTF-8 JSON, without `browser`).
    /// `mark` records the time as the last backup (for the "back up your data" reminder).
    pub fn export_backup(&self, mark: bool) -> anyhow::Result<Vec<u8>> {
        let conn = self.conn.lock();
        let tables = user_tables(&conn)?;
        let profile = crate::get_profile_in(&conn)?;
        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        buf.extend_from_slice(b"{\"format\":");
        serde_json::to_writer(&mut buf, FORMAT)?;
        buf.extend_from_slice(b",\"format_version\":");
        serde_json::to_writer(&mut buf, &FORMAT_VERSION)?;
        buf.extend_from_slice(b",\"schema_version\":");
        serde_json::to_writer(&mut buf, &schema_version(&conn)?)?;
        buf.extend_from_slice(b",\"app_version\":");
        serde_json::to_writer(&mut buf, env!("CARGO_PKG_VERSION"))?;
        buf.extend_from_slice(b",\"created_at\":");
        serde_json::to_writer(&mut buf, &now_iso(&conn)?)?;
        buf.extend_from_slice(b",\"profile\":");
        serde_json::to_writer(&mut buf, &profile)?;
        buf.extend_from_slice(b",\"tables\":{");
        for (ti, table) in tables.iter().enumerate() {
            if ti > 0 {
                buf.push(b',');
            }
            serde_json::to_writer(&mut buf, table)?;
            buf.extend_from_slice(b":[");
            let mut stmt = conn.prepare(&format!("SELECT * FROM {} ORDER BY rowid", q(table)))
                .or_else(|_| conn.prepare(&format!("SELECT * FROM {}", q(table))))?;
            let names: Vec<String> = stmt.column_names().into_iter().map(str::to_string).collect();
            let mut rows = stmt.query([])?;
            let mut first = true;
            while let Some(row) = rows.next()? {
                let mut obj = Map::new();
                for (i, name) in names.iter().enumerate() {
                    obj.insert(name.clone(), sql_to_json(row.get_ref(i)?));
                }
                if !first {
                    buf.push(b',');
                }
                first = false;
                serde_json::to_writer(&mut buf, &obj)?;
                if buf.len() > MAX_BACKUP_BYTES {
                    return Err(BackupError::TooLarge.into());
                }
            }
            buf.push(b']');
        }
        buf.extend_from_slice(b"}}");
        if mark {
            set_meta_now(&conn, "last_backup_at")?;
        }
        Ok(buf)
    }

    /// Last backup / restore / sync times and per-table row counts.
    pub fn backup_status(&self) -> anyhow::Result<BackupStatus> {
        let conn = self.conn.lock();
        let mut tables = Vec::new();
        let mut total = 0u64;
        for name in user_tables(&conn)? {
            let rows = count_rows(&conn, &name)?;
            total += rows;
            tables.push(TableCount { name, rows });
        }
        Ok(BackupStatus {
            last_backup_at: get_meta(&conn, "last_backup_at")?,
            last_restore_at: get_meta(&conn, "last_restore_at")?,
            last_sync_at: get_meta(&conn, "last_sync_at")?,
            tables,
            total_rows: total,
            schema_version: schema_version(&conn)?,
            format_version: FORMAT_VERSION,
        })
    }

    /// What importing `file` would touch: per-table counts, compatibility warnings.
    pub fn preview_backup(&self, file: &BackupFile) -> anyhow::Result<BackupPreview> {
        let conn = self.conn.lock();
        let current = user_tables(&conn)?;
        let infos = table_infos(&conn)?;
        let mut warnings = Vec::new();
        let current_schema = schema_version(&conn)?;
        if file.schema_version > current_schema {
            warn(&mut warnings, "newer_schema", None, None, file.schema_version.max(0) as u64);
        }
        let mut tables = Vec::new();
        let mut total = 0u64;
        for (name, rows) in &file.tables {
            let known = current.contains(name);
            let n = rows.len() as u64;
            total += n;
            if known {
                if let Some(info) = infos.iter().find(|t| &t.name == name) {
                    for col in unknown_columns(info, rows) {
                        warn(&mut warnings, "unknown_column", Some(name), Some(&col), 0);
                    }
                }
            } else {
                warn(&mut warnings, "unknown_table", Some(name), None, n);
            }
            tables.push(PreviewTable {
                name: name.clone(),
                rows: n,
                known,
                current_rows: if known { Some(count_rows(&conn, name)?) } else { None },
            });
        }
        let (browser, dropped) = sanitize_browser(&file.browser);
        if dropped {
            warn(&mut warnings, "browser_dropped", None, None, 0);
        }
        let profile_name = file
            .profile
            .get("name")
            .and_then(Value::as_str)
            .map(|s| s.chars().take(100).collect::<String>())
            .filter(|s| !s.trim().is_empty());
        Ok(BackupPreview {
            format_version: file.format_version,
            schema_version: file.schema_version,
            current_schema_version: current_schema,
            app_version: file.app_version.chars().take(40).collect(),
            created_at: file.created_at.chars().take(40).collect(),
            profile_name,
            tables,
            total_rows: total,
            warnings,
            browser,
        })
    }

    /// Imports `file` in one transaction. See the module docs for the merge rules.
    pub fn import_backup(&self, file: &BackupFile, mode: ImportMode, source: ImportSource) -> anyhow::Result<ImportReport> {
        let mut conn = self.conn.lock();
        let infos = table_infos(&conn)?;
        let mut report = ImportReport { mode: mode.as_str().into(), ..Default::default() };
        let current_schema = schema_version(&conn)?;
        if file.schema_version > current_schema {
            warn(&mut report.warnings, "newer_schema", None, None, file.schema_version.max(0) as u64);
        }
        for (name, rows) in &file.tables {
            match infos.iter().find(|t| &t.name == name) {
                Some(info) => {
                    for col in unknown_columns(info, rows) {
                        warn(&mut report.warnings, "unknown_column", Some(name), Some(&col), 0);
                    }
                }
                None => warn(&mut report.warnings, "unknown_table", Some(name), None, rows.len() as u64),
            }
        }

        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        match mode {
            ImportMode::Replace => replace_all(&tx, &infos, file, &mut report)?,
            ImportMode::Merge => merge_all(&tx, &infos, file, &mut report)?,
        }
        tx.execute_batch("INSERT OR IGNORE INTO profile (id) VALUES (1)")?;
        // Games from older backups have no thumbnail position yet.
        crate::game_fen::backfill(&tx)?;
        set_meta_now(&tx, if source == ImportSource::Sync { "last_sync_at" } else { "last_restore_at" })?;
        tx.commit().context("committing backup import")?;

        for t in &report.tables {
            if t.failed > 0 {
                report.warnings.push(BackupWarning {
                    code: "rows_failed".into(),
                    table: Some(t.name.clone()),
                    column: None,
                    count: t.failed,
                });
            }
        }
        report.inserted = report.tables.iter().map(|t| t.inserted).sum();
        report.updated = report.tables.iter().map(|t| t.updated).sum();
        report.skipped = report.tables.iter().map(|t| t.skipped).sum();
        report.failed = report.tables.iter().map(|t| t.failed).sum();
        let (browser, dropped) = sanitize_browser(&file.browser);
        if dropped {
            warn(&mut report.warnings, "browser_dropped", None, None, 0);
        }
        report.browser = browser;
        Ok(report)
    }
}

/// Columns present in the backup rows but not in the local table (sorted).
fn unknown_columns(info: &TableInfo, rows: &[Map<String, Value>]) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for row in rows {
        for k in row.keys() {
            if !info.cols.iter().any(|c| c == k) && seen.insert(k.as_str()) {
                out.push(k.clone());
            }
        }
    }
    out.sort();
    out
}

/// Known columns of `row`, in table order, with their SQL values.
fn row_values(info: &TableInfo, row: &Map<String, Value>, skip: Option<&str>) -> (Vec<String>, Vec<SqlValue>) {
    let mut cols = Vec::new();
    let mut vals = Vec::new();
    for c in &info.cols {
        if Some(c.as_str()) == skip {
            continue;
        }
        if let Some(v) = row.get(c) {
            cols.push(c.clone());
            vals.push(json_to_sql(v));
        }
    }
    (cols, vals)
}

fn insert_sql(table: &str, cols: &[String]) -> String {
    if cols.is_empty() {
        return format!("INSERT INTO {} DEFAULT VALUES", q(table));
    }
    let names: Vec<String> = cols.iter().map(|c| q(c)).collect();
    let marks: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
    format!("INSERT INTO {} ({}) VALUES ({})", q(table), names.join(", "), marks.join(", "))
}

fn replace_all(tx: &rusqlite::Transaction<'_>, infos: &[TableInfo], file: &BackupFile, report: &mut ImportReport) -> anyhow::Result<()> {
    tx.execute_batch("PRAGMA defer_foreign_keys = ON")?;
    for info in infos {
        tx.execute(&format!("DELETE FROM {}", q(&info.name)), [])?;
    }
    for info in infos {
        let Some(rows) = file.tables.get(&info.name) else { continue };
        let mut tr = TableReport { name: info.name.clone(), ..Default::default() };
        for row in rows {
            let (cols, vals) = row_values(info, row, None);
            let mut stmt = tx.prepare_cached(&insert_sql(&info.name, &cols))?;
            match stmt.execute(params_from_iter(vals.iter())) {
                Ok(_) => tr.inserted += 1,
                Err(e) if is_constraint(&e) => tr.failed += 1,
                Err(e) => return Err(e.into()),
            }
        }
        report.tables.push(tr);
    }
    drop_dangling_references(tx, report)?;
    Ok(())
}

/// Deletes rows whose foreign keys point nowhere (so the deferred check at COMMIT passes).
fn drop_dangling_references(tx: &rusqlite::Transaction<'_>, report: &mut ImportReport) -> anyhow::Result<()> {
    for _round in 0..8 {
        let bad: Vec<(String, Option<i64>)> = {
            let mut stmt = tx.prepare("SELECT \"table\", rowid FROM pragma_foreign_key_check")?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        if bad.is_empty() {
            return Ok(());
        }
        let mut removed = 0;
        for (table, rowid) in bad {
            let Some(rowid) = rowid else { continue };
            removed += tx.execute(&format!("DELETE FROM {} WHERE rowid = ?1", q(&table)), [rowid])?;
            match report.tables.iter_mut().find(|t| t.name == table) {
                Some(t) => {
                    t.inserted = t.inserted.saturating_sub(1);
                    t.failed += 1;
                }
                None => report.tables.push(TableReport { name: table, failed: 1, ..Default::default() }),
            }
        }
        if removed == 0 {
            break;
        }
    }
    Ok(())
}

fn merge_all(tx: &rusqlite::Transaction<'_>, infos: &[TableInfo], file: &BackupFile, report: &mut ImportReport) -> anyhow::Result<()> {
    // old id -> new id, per rowid-keyed table.
    let mut id_maps: HashMap<String, HashMap<i64, i64>> = HashMap::new();
    for idx in topo_order(infos) {
        let info = &infos[idx];
        let Some(rows) = file.tables.get(&info.name) else { continue };
        let mut tr = TableReport { name: info.name.clone(), ..Default::default() };
        if info.name == "profile" {
            if let Some(row) = rows.first() {
                if merge_profile(tx, row)? {
                    tr.updated += 1;
                } else {
                    tr.skipped += 1;
                }
            }
            report.tables.push(tr);
            continue;
        }
        // Remap references to rowid-keyed parents imported earlier.
        let remapped: Vec<Map<String, Value>> = rows
            .iter()
            .map(|row| {
                let mut row = row.clone();
                for (col, parent) in &info.refs {
                    if let (Some(map), Some(old)) = (id_maps.get(parent), row.get(col).and_then(Value::as_i64)) {
                        if let Some(new) = map.get(&old) {
                            row.insert(col.clone(), Value::from(*new));
                        }
                    }
                }
                row
            })
            .collect();
        if info.pk.is_empty() || info.rowid_pk.is_some() {
            let map = merge_by_content(tx, info, &remapped, &mut tr)?;
            if info.rowid_pk.is_some() {
                id_maps.insert(info.name.clone(), map);
            }
        } else {
            merge_by_key(tx, info, &remapped, &mut tr)?;
        }
        report.tables.push(tr);
    }
    drop_dangling_references(tx, report)?;
    // Merged attempt histories can hold more solves than either profile counted.
    if infos.iter().any(|t| t.name == "puzzle_attempts") && infos.iter().any(|t| t.name == "profile") {
        tx.execute_batch(
            "UPDATE profile SET \
             puzzles_solved = MAX(puzzles_solved, (SELECT COUNT(*) FROM puzzle_attempts WHERE solved != 0)), \
             puzzles_failed = MAX(puzzles_failed, (SELECT COUNT(*) FROM puzzle_attempts WHERE solved = 0)) \
             WHERE id = 1",
        )?;
    }
    Ok(())
}

/// Columns that identify a row's content (for de-duplication).
fn content_key_cols(info: &TableInfo, rows: &[Map<String, Value>]) -> Vec<String> {
    let preferred: &[&str] = if info.name == "games" { &["start_fen", "moves", "created_at"] } else { &[] };
    let present = |c: &str| rows.iter().any(|r| r.contains_key(c));
    if !preferred.is_empty() && preferred.iter().all(|c| info.cols.iter().any(|x| x == c) && present(c)) {
        return preferred.iter().map(|c| c.to_string()).collect();
    }
    info.cols
        .iter()
        .filter(|c| Some(c.as_str()) != info.rowid_pk.as_deref() && c.as_str() != "updated_at" && present(c))
        .cloned()
        .collect()
}

/// Rowid-keyed / key-less tables: insert rows whose content is not already present.
/// Returns old id -> local id for every row in the backup.
fn merge_by_content(
    tx: &rusqlite::Transaction<'_>,
    info: &TableInfo,
    rows: &[Map<String, Value>],
    tr: &mut TableReport,
) -> anyhow::Result<HashMap<i64, i64>> {
    let key_cols = content_key_cols(info, rows);
    let pk = info.rowid_pk.as_deref();
    // Existing content keys -> local rowid.
    let mut existing: HashMap<String, i64> = HashMap::new();
    {
        let select: Vec<String> = key_cols.iter().map(|c| q(c)).collect();
        let sql = format!(
            "SELECT rowid{}{} FROM {}",
            if select.is_empty() { "" } else { ", " },
            select.join(", "),
            q(&info.name)
        );
        let mut stmt = tx.prepare(&sql)?;
        let mut rs = stmt.query([])?;
        while let Some(r) = rs.next()? {
            let id: i64 = r.get(0)?;
            let mut key = String::new();
            for i in 0..key_cols.len() {
                key_part(&SqlValue::from(r.get_ref(i + 1)?), &mut key);
            }
            existing.entry(key).or_insert(id);
        }
    }
    let mut map = HashMap::new();
    for row in rows {
        let mut key = String::new();
        for c in &key_cols {
            key_part(&row.get(c).map(json_to_sql).unwrap_or(SqlValue::Null), &mut key);
        }
        let old_id = pk.and_then(|p| row.get(p)).and_then(Value::as_i64);
        if let Some(&local) = existing.get(&key) {
            if let Some(old) = old_id {
                map.insert(old, local);
            }
            tr.skipped += 1;
            continue;
        }
        let (cols, vals) = row_values(info, row, pk);
        let mut stmt = tx.prepare_cached(&insert_sql(&info.name, &cols))?;
        match stmt.execute(params_from_iter(vals.iter())) {
            Ok(_) => {
                let new_id = tx.last_insert_rowid();
                if let Some(old) = old_id {
                    map.insert(old, new_id);
                }
                existing.insert(key, new_id);
                tr.inserted += 1;
            }
            Err(e) if is_constraint(&e) => tr.failed += 1,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(map)
}

/// Natural-key tables: insert missing rows; replace a local row only when the backup's
/// `updated_at` is newer.
fn merge_by_key(tx: &rusqlite::Transaction<'_>, info: &TableInfo, rows: &[Map<String, Value>], tr: &mut TableReport) -> anyhow::Result<()> {
    let has_updated = info.cols.iter().any(|c| c == "updated_at");
    let where_pk: Vec<String> = info.pk.iter().enumerate().map(|(i, c)| format!("{} IS ?{}", q(c), i + 1)).collect();
    let where_pk = where_pk.join(" AND ");
    let probe = format!(
        "SELECT {} FROM {} WHERE {where_pk}",
        if has_updated { "updated_at" } else { "NULL" },
        q(&info.name)
    );
    for row in rows {
        if info.pk.iter().any(|c| !row.contains_key(c)) {
            tr.failed += 1;
            continue;
        }
        let pk_vals: Vec<SqlValue> = info.pk.iter().map(|c| row.get(c).map(json_to_sql).unwrap_or(SqlValue::Null)).collect();
        let local: Option<Option<String>> = tx
            .prepare_cached(&probe)?
            .query_row(params_from_iter(pk_vals.iter()), |r| r.get::<_, Option<String>>(0))
            .optional()?;
        match local {
            None => {
                let (cols, vals) = row_values(info, row, None);
                let mut stmt = tx.prepare_cached(&insert_sql(&info.name, &cols))?;
                match stmt.execute(params_from_iter(vals.iter())) {
                    Ok(_) => tr.inserted += 1,
                    Err(e) if is_constraint(&e) => tr.failed += 1,
                    Err(e) => return Err(e.into()),
                }
            }
            Some(local_updated) => {
                let incoming = row.get("updated_at").and_then(Value::as_str);
                let newer = has_updated && matches!((incoming, local_updated.as_deref()), (Some(a), Some(b)) if a > b);
                if !newer {
                    tr.skipped += 1;
                    continue;
                }
                let (cols, vals) = row_values(info, row, None);
                let sets: Vec<(String, SqlValue)> = cols
                    .into_iter()
                    .zip(vals)
                    .filter(|(c, _)| !info.pk.contains(c))
                    .collect();
                if sets.is_empty() {
                    tr.skipped += 1;
                    continue;
                }
                let n = info.pk.len();
                let assign: Vec<String> = sets.iter().enumerate().map(|(i, (c, _))| format!("{} = ?{}", q(c), n + i + 1)).collect();
                let sql = format!("UPDATE {} SET {} WHERE {where_pk}", q(&info.name), assign.join(", "));
                let all: Vec<SqlValue> = pk_vals.into_iter().chain(sets.into_iter().map(|(_, v)| v)).collect();
                match tx.prepare_cached(&sql)?.execute(params_from_iter(all.iter())) {
                    Ok(_) => tr.updated += 1,
                    Err(e) if is_constraint(&e) => tr.failed += 1,
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    Ok(())
}

/// Profile merge: best-of counters, the most recently active side's rating and streak,
/// local name / avatar / settings (unless the local profile was never personalised).
fn merge_profile(tx: &rusqlite::Transaction<'_>, row: &Map<String, Value>) -> anyhow::Result<bool> {
    let local = tx.query_row(
        "SELECT name, avatar, puzzle_rating, puzzle_rd, rush_best, puzzles_solved, puzzles_failed, streak_days, last_active \
         FROM profile WHERE id = 1",
        [],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, String>(8)?,
            ))
        },
    ).optional()?;
    let Some((name, avatar, rating, rd, rush, solved, failed, streak, last)) = local else { return Ok(false) };
    let s = |k: &str| row.get(k).and_then(Value::as_str).map(str::to_string);
    let i = |k: &str| row.get(k).and_then(Value::as_i64).map(|v| v.clamp(0, 10_000_000));
    let f = |k: &str| row.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());

    let (mut n_name, mut n_avatar) = (name.clone(), avatar.clone());
    if name == "Player" {
        if let Some(inc) = s("name").map(|v| v.trim().chars().take(100).collect::<String>()).filter(|v| !v.is_empty()) {
            n_name = inc;
            if let Some(a) = s("avatar").filter(|a| !a.is_empty() && a.chars().count() <= 16) {
                n_avatar = a;
            }
        }
    }
    let n_rush = rush.max(i("rush_best").unwrap_or(0));
    let n_solved = solved.max(i("puzzles_solved").unwrap_or(0));
    let n_failed = failed.max(i("puzzles_failed").unwrap_or(0));
    let inc_last = s("last_active").unwrap_or_default();
    let (mut n_rating, mut n_rd, mut n_streak, mut n_last) = (rating, rd, streak, last.clone());
    if inc_last.len() <= 40 && inc_last > last {
        n_rating = f("puzzle_rating").map(|v| v.clamp(100.0, 4000.0)).unwrap_or(rating);
        n_rd = f("puzzle_rd").map(|v| v.clamp(0.0, 1000.0)).unwrap_or(rd);
        n_streak = i("streak_days").unwrap_or(streak);
        n_last = inc_last;
    }
    let changed = n_name != name
        || n_avatar != avatar
        || n_rush != rush
        || n_solved != solved
        || n_failed != failed
        || n_last != last
        || n_rating != rating
        || n_rd != rd
        || n_streak != streak;
    if changed {
        tx.execute(
            "UPDATE profile SET name = ?1, avatar = ?2, puzzle_rating = ?3, puzzle_rd = ?4, rush_best = ?5, \
             puzzles_solved = ?6, puzzles_failed = ?7, streak_days = ?8, last_active = ?9 WHERE id = 1",
            params![n_name, n_avatar, n_rating, n_rd, n_rush, n_solved, n_failed, n_streak, n_last],
        )?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewGame, ProfilePatch};

    fn seed(s: &Store) {
        s.create_game(&NewGame {
            white: "Me".into(),
            black: "Bot".into(),
            result: "1-0".into(),
            moves: vec!["e2e4".into(), "e7e5".into(), "g1f3".into()],
            user_color: Some("white".into()),
            bot_id: Some("martin".into()),
            tags: vec!["fun".into()],
            ..Default::default()
        })
        .unwrap();
        s.create_game(&NewGame { moves: vec!["d2d4".into()], ..Default::default() }).unwrap();
        s.record_puzzle_attempt("abc", 1300, true, 4000).unwrap();
        s.record_puzzle_attempt("def", 1250, false, 9000).unwrap();
        s.set_lesson_progress("basics", "l1", true).unwrap();
        s.log_activity("puzzle", 2).unwrap();
        s.update_profile(&ProfilePatch { name: Some("Ana".into()), ..Default::default() }).unwrap();
    }

    fn tables_of(bytes: &[u8]) -> BTreeMap<String, Vec<Map<String, Value>>> {
        parse_backup(bytes).unwrap().tables
    }

    #[test]
    fn export_lists_all_user_tables() {
        let s = Store::open_in_memory().unwrap();
        seed(&s);
        let bytes = s.export_backup(true).unwrap();
        let f = parse_backup(&bytes).unwrap();
        assert_eq!(f.format, FORMAT);
        assert_eq!(f.format_version, FORMAT_VERSION);
        assert!(f.tables.contains_key("games"));
        assert!(f.tables.contains_key("activity"));
        assert!(!f.tables.contains_key(META_TABLE));
        assert!(!f.tables.keys().any(|k| k.starts_with("sqlite_")));
        assert_eq!(f.tables["games"].len(), 2);
        assert_eq!(f.profile["name"], "Ana");
        assert!(s.backup_status().unwrap().last_backup_at.is_some());
    }

    #[test]
    fn round_trip_replace_is_identical() {
        let a = Store::open_in_memory().unwrap();
        seed(&a);
        let bytes = a.export_backup(false).unwrap();

        let b = Store::open_in_memory().unwrap();
        b.create_game(&NewGame { moves: vec!["c2c4".into()], ..Default::default() }).unwrap();
        let file = parse_backup(&bytes).unwrap();
        let rep = b.import_backup(&file, ImportMode::Replace, ImportSource::File).unwrap();
        assert_eq!(rep.failed, 0, "{rep:?}");
        assert_eq!(tables_of(&b.export_backup(false).unwrap()), tables_of(&bytes));
        assert_eq!(b.get_profile().unwrap().name, "Ana");
        assert!(b.backup_status().unwrap().last_restore_at.is_some());
    }

    #[test]
    fn merge_is_idempotent_and_remaps_ids() {
        let a = Store::open_in_memory().unwrap();
        seed(&a);
        let file = parse_backup(&a.export_backup(false).unwrap()).unwrap();

        let b = Store::open_in_memory().unwrap();
        // Local data that collides on ids.
        b.create_game(&NewGame { moves: vec!["c2c4".into()], ..Default::default() }).unwrap();
        b.record_puzzle_attempt("zzz", 1400, true, 1000).unwrap();
        let first = b.import_backup(&file, ImportMode::Merge, ImportSource::Sync).unwrap();
        assert_eq!(first.failed, 0, "{first:?}");
        let games = |s: &Store| s.list_games(&Default::default()).unwrap().len();
        assert_eq!(games(&b), 3);
        let snapshot = tables_of(&b.export_backup(false).unwrap());

        let second = b.import_backup(&file, ImportMode::Merge, ImportSource::Sync).unwrap();
        assert_eq!(second.inserted, 0, "{second:?}");
        assert_eq!(second.updated, 0, "{second:?}");
        assert_eq!(tables_of(&b.export_backup(false).unwrap()), snapshot);
        assert_eq!(b.get_profile().unwrap().name, "Ana", "default local name takes the backup's");
        assert_eq!(b.get_profile().unwrap().puzzles_solved, 2);
        assert!(b.backup_status().unwrap().last_sync_at.is_some());

        // rating_history.attempt_id points at the remapped puzzle attempts.
        let conn = b.conn.lock();
        let dangling: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rating_history WHERE attempt_id IS NOT NULL \
                 AND attempt_id NOT IN (SELECT id FROM puzzle_attempts)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dangling, 0);
    }

    #[test]
    fn unknown_table_and_column_are_skipped() {
        let a = Store::open_in_memory().unwrap();
        seed(&a);
        let mut file = parse_backup(&a.export_backup(false).unwrap()).unwrap();
        let mut row = Map::new();
        row.insert("x".into(), Value::from(1));
        file.tables.insert("from_the_future".into(), vec![row]);
        for g in file.tables.get_mut("games").unwrap() {
            g.insert("hologram".into(), Value::from("yes"));
        }
        let b = Store::open_in_memory().unwrap();
        let preview = b.preview_backup(&file).unwrap();
        assert!(preview.tables.iter().any(|t| t.name == "from_the_future" && !t.known));
        assert!(preview.warnings.iter().any(|w| w.code == "unknown_table"));
        assert!(preview.warnings.iter().any(|w| w.code == "unknown_column" && w.column.as_deref() == Some("hologram")));
        let rep = b.import_backup(&file, ImportMode::Merge, ImportSource::File).unwrap();
        assert!(rep.warnings.iter().any(|w| w.code == "unknown_table"));
        assert_eq!(b.list_games(&Default::default()).unwrap().len(), 2);
    }

    #[test]
    fn rejects_foreign_and_future_files() {
        assert!(matches!(parse_backup(b"not json"), Err(BackupError::Malformed(_))));
        assert_eq!(parse_backup(b"{\"a\":1}"), Err(BackupError::NotABackup));
        assert_eq!(parse_backup(b"[1,2]"), Err(BackupError::NotABackup));
        let future = br#"{"format":"grandmentor-backup","format_version":99,"tables":{}}"#;
        assert_eq!(
            parse_backup(future),
            Err(BackupError::UnsupportedVersion { found: 99, supported: FORMAT_VERSION })
        );
    }

    #[test]
    fn browser_settings_are_sanitized() {
        let v = serde_json::json!({
            "grandmentor.settings.v1": "{\"theme\":\"light\"}",
            "gm.endgames.v1": "[]",
            "evil": "x",
            "grandmentor.obj": {"a": 1}
        });
        let (out, dropped) = sanitize_browser(&v);
        assert!(dropped);
        assert_eq!(out.as_object().unwrap().len(), 2);
    }

    #[test]
    fn values_round_trip() {
        for v in [SqlValue::Null, SqlValue::Integer(-5), SqlValue::Real(1.5), SqlValue::Text("é".into()), SqlValue::Blob(vec![0, 255, 16])] {
            let j = sql_to_json(ValueRef::from(&v));
            assert_eq!(json_to_sql(&j), v);
        }
    }
}
