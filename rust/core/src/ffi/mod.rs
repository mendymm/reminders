//! The surface exposed to Kotlin through UniFFI.
//!
//! [`ReminderStore`] is the one object: it owns a [`LocalReminders`] over a
//! [`SqliteStorage`] behind a mutex, so calls are serialized. Calls block, so
//! Kotlin makes them off the UI thread. Every call that needs the time takes a
//! [`CallContext`]. Ids are made with [`new_reminder_id`] and
//! [`new_resolution_id`], so Kotlin never invents them.
//!
//! The types in `types` mirror the model one to one. Anything Kotlin passes in
//! goes through the model's constructors, and bad input returns
//! [`ReminderError::InvalidInput`].

mod error;
mod types;

#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub use error::ReminderError;
pub use types::*;

use crate::commands::{self as cmd, LocalReminders, Reminders};
use crate::model as domain;
use crate::store::SqliteStorage;
use types::{timestamp_from_ms, uuid_from_text};

/// A new reminder id, a UUIDv7 stamped with `now_ms`.
#[uniffi::export]
pub fn new_reminder_id(now_ms: i64) -> Result<String, ReminderError> {
    Ok(domain::ReminderId::new(timestamp_from_ms(now_ms)?)
        .0
        .to_string())
}

/// A new resolution id, a UUIDv7 stamped with `now_ms`.
#[uniffi::export]
pub fn new_resolution_id(now_ms: i64) -> Result<String, ReminderError> {
    Ok(domain::ResolutionId::new(timestamp_from_ms(now_ms)?)
        .0
        .to_string())
}

/// Writes a consistent copy of the database at `source` to a new file at
/// `destination`, which must not exist. The copy includes everything committed,
/// including what a copy of the file alone would miss. The source is left as it is
/// and is not migrated.
#[uniffi::export]
pub fn copy_database(source: String, destination: String) -> Result<(), ReminderError> {
    crate::store::copy_database(Path::new(&source), Path::new(&destination))
        .map_err(storage_error)
}

#[derive(uniffi::Object)]
pub struct ReminderStore {
    inner: Mutex<LocalReminders<SqliteStorage>>,
}

impl ReminderStore {
    fn from_storage(storage: SqliteStorage) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(LocalReminders::new(storage)),
        })
    }

    /// Each command commits atomically, so a panic in an earlier call cannot have
    /// left the data half written. A poisoned lock is safe to take over.
    fn lock(&self) -> MutexGuard<'_, LocalReminders<SqliteStorage>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn storage_error(error: cmd::StorageError) -> ReminderError {
    cmd::CommandError::Storage(error).into()
}

#[uniffi::export]
impl ReminderStore {
    /// Opens the database at `path`, creating it if needed.
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, ReminderError> {
        SqliteStorage::open(path)
            .map(Self::from_storage)
            .map_err(storage_error)
    }

    /// A store that lives only in memory, for previews and tests.
    #[uniffi::constructor]
    pub fn open_in_memory() -> Result<Arc<Self>, ReminderError> {
        SqliteStorage::open_in_memory()
            .map(Self::from_storage)
            .map_err(storage_error)
    }

    pub fn create(
        &self,
        ctx: CallContext,
        id: String,
        draft: ReminderDraft,
    ) -> Result<ReminderState, ReminderError> {
        let ctx = ctx.into_domain()?;
        let id = domain::ReminderId(uuid_from_text(&id)?);
        let draft = cmd::ReminderDraft::try_from(draft)?;
        Ok(self.lock().create(&ctx, id, draft)?.into())
    }

    /// Applies `edit` if the reminder is still at `base_revision`.
    pub fn edit(
        &self,
        ctx: CallContext,
        id: String,
        base_revision: u64,
        edit: ReminderEdit,
    ) -> Result<ReminderState, ReminderError> {
        let ctx = ctx.into_domain()?;
        let id = domain::ReminderId(uuid_from_text(&id)?);
        let edit = cmd::ReminderEdit::try_from(edit)?;
        Ok(self.lock().edit(&ctx, id, base_revision, edit)?.into())
    }

    pub fn delete(
        &self,
        ctx: CallContext,
        id: String,
        base_revision: u64,
    ) -> Result<(), ReminderError> {
        let ctx = ctx.into_domain()?;
        let id = domain::ReminderId(uuid_from_text(&id)?);
        Ok(self.lock().delete(&ctx, id, base_revision)?)
    }

    /// Records a response to a reminder. `resolution_id` comes from
    /// [`new_resolution_id`].
    pub fn resolve(
        &self,
        ctx: CallContext,
        id: String,
        resolution_id: String,
        outcome: Outcome,
        note: Option<String>,
    ) -> Result<Resolved, ReminderError> {
        let ctx = ctx.into_domain()?;
        let id = domain::ReminderId(uuid_from_text(&id)?);
        let resolution_id = domain::ResolutionId(uuid_from_text(&resolution_id)?);
        let outcome = domain::Outcome::try_from(outcome)?;
        Ok(self
            .lock()
            .resolve(&ctx, id, resolution_id, outcome, note)?
            .into())
    }

    pub fn undo(
        &self,
        ctx: CallContext,
        id: String,
        resolution_id: String,
    ) -> Result<(), ReminderError> {
        let ctx = ctx.into_domain()?;
        let id = domain::ReminderId(uuid_from_text(&id)?);
        let resolution_id = domain::ResolutionId(uuid_from_text(&resolution_id)?);
        Ok(self.lock().undo(&ctx, id, resolution_id)?)
    }

    pub fn get(&self, id: String) -> Result<ReminderState, ReminderError> {
        let id = domain::ReminderId(uuid_from_text(&id)?);
        Ok(self.lock().get(id)?.into())
    }

    /// Every reminder that is not deleted, oldest first.
    pub fn list(&self) -> Result<Vec<ReminderState>, ReminderError> {
        Ok(self
            .lock()
            .list()?
            .into_iter()
            .map(ReminderState::from)
            .collect())
    }

    /// Pending slots in the order they fire, at most `limit`.
    pub fn upcoming(&self, ctx: CallContext, limit: u32) -> Result<UpcomingList, ReminderError> {
        let ctx = ctx.into_domain()?;
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        Ok(self.lock().upcoming(&ctx, limit)?.into())
    }

    /// The next slot of each reminder that has not fired yet, in the order they
    /// fire, at most `limit`. Set alarms from this. A slot that already passed
    /// unresolved is skipped, so an ignored slot never hides the ones after it.
    pub fn alarms(&self, ctx: CallContext, limit: u32) -> Result<UpcomingList, ReminderError> {
        let ctx = ctx.into_domain()?;
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        Ok(self.lock().alarms(&ctx, limit)?.into())
    }

    /// A reminder's resolutions, newest first, without retracted ones.
    pub fn history(&self, id: String) -> Result<Vec<Resolution>, ReminderError> {
        let id = domain::ReminderId(uuid_from_text(&id)?);
        Ok(self
            .lock()
            .history(id)?
            .into_iter()
            .map(Resolution::from)
            .collect())
    }
}
