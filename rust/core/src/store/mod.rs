//! SQLite implementation of the command layer's [`Storage`] port, on plain SQL
//! through rusqlite.
//!
//! The schema is built by the migrations in `migrations`, tracked with
//! `PRAGMA user_version`. A database from a newer app is refused. Every commit is
//! one immediate transaction, so a refused [`WriteSet`] changes nothing.

mod codec;
mod migrations;
#[cfg(test)]
pub(crate) mod tests;

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::commands::{
    ProgressWrite, ResolutionWrite, Storage, StorageError, WriteSet, check_expect,
};
use crate::model::{Progress, Reminder, ReminderId, Resolution, ResolutionId};
use codec::{RawProgress, RawReminder, RawResolution, id_bytes};
use migrations::MIGRATIONS;

const REMINDER_COLUMNS: &str =
    "id, title, notes, plan, archived_at, revision, created_at, updated_at, deleted_at";
const PROGRESS_COLUMNS: &str = "reminder_id, kind, done_at, through, handled, previous_through, \
     previous_handled, last_resolution, revision, updated_at";
const RESOLUTION_COLUMNS: &str = "id, reminder_id, resolved_at, outcome, slot, note, deleted_at";

const UPSERT_REMINDER: &str = "INSERT INTO reminders \
     (id, title, notes, plan, archived_at, revision, created_at, updated_at, deleted_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
     ON CONFLICT (id) DO UPDATE SET \
     title = excluded.title, notes = excluded.notes, plan = excluded.plan, \
     archived_at = excluded.archived_at, revision = excluded.revision, \
     created_at = excluded.created_at, updated_at = excluded.updated_at, \
     deleted_at = excluded.deleted_at";

const UPSERT_PROGRESS: &str = "INSERT INTO progress \
     (reminder_id, kind, done_at, through, handled, previous_through, previous_handled, \
      last_resolution, revision, updated_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
     ON CONFLICT (reminder_id) DO UPDATE SET \
     kind = excluded.kind, done_at = excluded.done_at, through = excluded.through, \
     handled = excluded.handled, previous_through = excluded.previous_through, \
     previous_handled = excluded.previous_handled, \
     last_resolution = excluded.last_resolution, revision = excluded.revision, \
     updated_at = excluded.updated_at";

const UPSERT_RESOLUTION: &str = "INSERT INTO resolutions \
     (id, reminder_id, resolved_at, outcome, slot, note, deleted_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
     ON CONFLICT (id) DO UPDATE SET \
     reminder_id = excluded.reminder_id, resolved_at = excluded.resolved_at, \
     outcome = excluded.outcome, slot = excluded.slot, note = excluded.note, \
     deleted_at = excluded.deleted_at";

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Backend(error.to_string())
    }
}

pub struct SqliteStorage {
    conn: Connection,
}

impl SqliteStorage {
    /// Opens the database at `path`, creating it and applying any migrations it
    /// is missing.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::from_connection(Connection::open(path)?)
    }

    /// A private in-memory database, gone when this is dropped.
    pub fn open_in_memory() -> Result<Self, StorageError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(mut conn: Connection) -> Result<Self, StorageError> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;",
        )?;
        migrate(&mut conn)?;
        Ok(Self { conn })
    }
}

fn migrate(conn: &mut Connection) -> Result<(), StorageError> {
    let applied: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let applied = usize::try_from(applied)
        .map_err(|_| StorageError::Corrupt("the schema version is negative".to_owned()))?;
    if applied > MIGRATIONS.len() {
        return Err(StorageError::Backend(format!(
            "the database is at schema version {applied}, newer than the {} this app knows",
            MIGRATIONS.len()
        )));
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", index + 1))?;
        tx.commit()?;
    }
    Ok(())
}

fn current_reminder_revision(
    tx: &Transaction<'_>,
    id: ReminderId,
) -> Result<Option<u64>, StorageError> {
    let revision: Option<i64> = tx
        .query_row(
            "SELECT revision FROM reminders WHERE id = ?1",
            params![id_bytes(id.0)],
            |row| row.get(0),
        )
        .optional()?;
    revision
        .map(|revision| {
            u64::try_from(revision)
                .map_err(|_| StorageError::Corrupt("a revision is negative".to_owned()))
        })
        .transpose()
}

fn current_progress_revision(
    tx: &Transaction<'_>,
    id: ReminderId,
) -> Result<Option<u64>, StorageError> {
    let revision: Option<i64> = tx
        .query_row(
            "SELECT revision FROM progress WHERE reminder_id = ?1",
            params![id_bytes(id.0)],
            |row| row.get(0),
        )
        .optional()?;
    revision
        .map(|revision| {
            u64::try_from(revision)
                .map_err(|_| StorageError::Corrupt("a revision is negative".to_owned()))
        })
        .transpose()
}

fn resolution_exists(tx: &Transaction<'_>, id: ResolutionId) -> Result<bool, StorageError> {
    Ok(tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM resolutions WHERE id = ?1)",
        params![id_bytes(id.0)],
        |row| row.get(0),
    )?)
}

fn put_reminder(tx: &Transaction<'_>, reminder: &Reminder) -> Result<(), StorageError> {
    let raw = RawReminder::encode(reminder)?;
    tx.execute(
        UPSERT_REMINDER,
        params![
            raw.id,
            raw.title,
            raw.notes,
            raw.plan,
            raw.archived_at,
            raw.revision,
            raw.created_at,
            raw.updated_at,
            raw.deleted_at,
        ],
    )?;
    Ok(())
}

fn put_progress(tx: &Transaction<'_>, progress: &Progress) -> Result<(), StorageError> {
    let raw = RawProgress::encode(progress)?;
    tx.execute(
        UPSERT_PROGRESS,
        params![
            raw.reminder_id,
            raw.kind,
            raw.done_at,
            raw.through,
            raw.handled,
            raw.previous_through,
            raw.previous_handled,
            raw.last_resolution,
            raw.revision,
            raw.updated_at,
        ],
    )?;
    Ok(())
}

fn put_resolution(tx: &Transaction<'_>, resolution: &Resolution) -> Result<(), StorageError> {
    let raw = RawResolution::encode(resolution);
    tx.execute(
        UPSERT_RESOLUTION,
        params![
            raw.id,
            raw.reminder_id,
            raw.resolved_at,
            raw.outcome,
            raw.slot,
            raw.note,
            raw.deleted_at,
        ],
    )?;
    Ok(())
}

impl Storage for SqliteStorage {
    fn reminder(&self, id: ReminderId) -> Result<Option<Reminder>, StorageError> {
        self.conn
            .query_row(
                &format!("SELECT {REMINDER_COLUMNS} FROM reminders WHERE id = ?1"),
                params![id_bytes(id.0)],
                RawReminder::from_row,
            )
            .optional()?
            .map(RawReminder::decode)
            .transpose()
    }

    fn reminders(&self) -> Result<Vec<Reminder>, StorageError> {
        let mut statement = self
            .conn
            .prepare(&format!("SELECT {REMINDER_COLUMNS} FROM reminders"))?;
        let raws = statement
            .query_map([], RawReminder::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        raws.into_iter().map(RawReminder::decode).collect()
    }

    fn progress(&self, id: ReminderId) -> Result<Option<Progress>, StorageError> {
        self.conn
            .query_row(
                &format!("SELECT {PROGRESS_COLUMNS} FROM progress WHERE reminder_id = ?1"),
                params![id_bytes(id.0)],
                RawProgress::from_row,
            )
            .optional()?
            .map(RawProgress::decode)
            .transpose()
    }

    fn resolution(&self, id: ResolutionId) -> Result<Option<Resolution>, StorageError> {
        self.conn
            .query_row(
                &format!("SELECT {RESOLUTION_COLUMNS} FROM resolutions WHERE id = ?1"),
                params![id_bytes(id.0)],
                RawResolution::from_row,
            )
            .optional()?
            .map(RawResolution::decode)
            .transpose()
    }

    fn resolutions(&self, id: ReminderId) -> Result<Vec<Resolution>, StorageError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {RESOLUTION_COLUMNS} FROM resolutions WHERE reminder_id = ?1 \
             ORDER BY resolved_at DESC, id DESC"
        ))?;
        let raws = statement
            .query_map(params![id_bytes(id.0)], RawResolution::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        raws.into_iter().map(RawResolution::decode).collect()
    }

    fn commit(&mut self, writes: WriteSet) -> Result<(), StorageError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Check everything first, so a refusal leaves nothing half written. The
        // transaction would roll back anyway; this keeps the errors the same as
        // every other `Storage`.
        if let Some((reminder, expect)) = &writes.reminder {
            check_expect(current_reminder_revision(&tx, reminder.id)?, *expect)?;
        }
        if let Some(ProgressWrite::Put(progress, expect)) = &writes.progress {
            check_expect(current_progress_revision(&tx, progress.reminder_id)?, *expect)?;
        }
        match &writes.resolution {
            Some(ResolutionWrite::Insert(resolution)) if resolution_exists(&tx, resolution.id)? => {
                return Err(StorageError::AlreadyExists);
            }
            Some(ResolutionWrite::Update(resolution))
                if !resolution_exists(&tx, resolution.id)? =>
            {
                return Err(StorageError::Missing);
            }
            _ => {}
        }

        if let Some((reminder, _)) = &writes.reminder {
            put_reminder(&tx, reminder)?;
        }
        match &writes.progress {
            Some(ProgressWrite::Put(progress, _)) => put_progress(&tx, progress)?,
            Some(ProgressWrite::Delete(id)) => {
                tx.execute(
                    "DELETE FROM progress WHERE reminder_id = ?1",
                    params![id_bytes(id.0)],
                )?;
            }
            None => {}
        }
        if let Some(ResolutionWrite::Insert(resolution) | ResolutionWrite::Update(resolution)) =
            &writes.resolution
        {
            put_resolution(&tx, resolution)?;
        }

        tx.commit()?;
        Ok(())
    }
}
