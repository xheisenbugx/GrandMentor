//! Quick-drill scores (coordinate vision, counting material, hanging pieces, ...).
//!
//! Tables are created by [`schema`], which runs inside schema migration v2 (see `lib.rs`).

#[allow(unused_imports)]
use crate::Store;

/// Creates this module's tables. Must be idempotent (`CREATE TABLE IF NOT EXISTS ...`).
pub(crate) fn schema(_tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    Ok(())
}
