//! The command layer: the API that the FFI, and later a network client, wrap.
//!
//! [`Reminders`] is the trait. [`LocalReminders`] implements it against a
//! [`Storage`]: it validates, applies the model's transitions and commits each
//! command as one atomic write-set. Commands take a [`Context`], so the core
//! never reads the clock or the device zone.
//!
//! Identifiers come from the caller, so a retried command carries the same id.
//! A repeated id fails with [`CommandError::AlreadyExists`], which a caller that
//! is retrying can treat as already applied.

mod local;
#[cfg(test)]
pub(crate) mod memory;
mod storage;
mod validate;

#[cfg(test)]
pub(crate) mod tests;

use std::fmt;

use jiff::Timestamp;
use jiff::tz::TimeZone;

pub use local::LocalReminders;
pub use storage::{
    Expect, ProgressWrite, ResolutionWrite, Storage, StorageError, WriteSet, check_expect,
};

use crate::instant::ZoneError;
use crate::model::{
    Alert, Outcome, Plan, Progress, Reminder, ReminderId, Resolution, ResolutionId, Slot, Title,
};
use crate::progress::TransitionError;

/// What a command needs to know about the world.
#[derive(Debug, Clone)]
pub struct Context {
    pub now: Timestamp,
    /// The zone the device is in now, for [`crate::model::Zone::Floating`].
    pub device_zone: TimeZone,
}

/// A new reminder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderDraft {
    pub title: Title,
    pub notes: String,
    pub plan: Plan,
}

/// Changes to a reminder. Fields left as `None` stay as they are.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReminderEdit {
    pub title: Option<Title>,
    pub notes: Option<String>,
    /// Replacing the plan keeps the reminder's progress when the plan stays the
    /// same kind (one-off, cadence or tracker) and resets it when the kind changes.
    pub plan: Option<Plan>,
    /// `Some(true)` archives, `Some(false)` restores.
    pub archived: Option<bool>,
}

/// A reminder with its progress. A tracker has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderState {
    pub reminder: Reminder,
    pub progress: Option<Progress>,
}

/// The result of resolving: the archived record, and the reminder's progress
/// afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub resolution: Resolution,
    pub progress: Option<Progress>,
}

/// A slot waiting to alert, with what the alarm layer needs to do so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upcoming {
    pub reminder_id: ReminderId,
    pub title: Title,
    pub slot: Slot,
    /// When the slot fires. Past for an overdue slot.
    pub at: Timestamp,
    pub alert: Alert,
}

/// Upcoming slots in the order they fire, plus reminders that could not be
/// scheduled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpcomingList {
    pub upcoming: Vec<Upcoming>,
    pub unresolved: Vec<Unresolved>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    pub reminder_id: ReminderId,
    pub reason: UnresolvedReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnresolvedReason {
    Zone(ZoneError),
    /// The reminder has a schedule but no progress row.
    MissingProgress,
}

/// A request the command layer refuses on its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    /// A fixed zone name the time zone database does not know.
    UnknownZone(String),
    /// A recurrence whose end is before its start.
    EndsBeforeStart,
    /// An outcome the reminder's plan cannot take: a skip of a one-off, or a
    /// slot on a plan with no slots.
    OutcomeNotAllowed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// No such reminder or resolution, or it is deleted.
    NotFound,
    /// The id is already in use.
    AlreadyExists,
    /// The reminder changed since the revision the caller based its edit on.
    Conflict { current: u64 },
    /// The reminder is archived. Restore it first.
    Archived,
    Invalid(Invalid),
    /// The slot is further ahead than the next pending slot.
    NotResolvable,
    /// Only the latest resolution that moved progress can be undone.
    NotLatest,
    Transition(TransitionError),
    Zone(ZoneError),
    /// The reminder has a schedule but no progress row.
    MissingProgress,
    Storage(StorageError),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "not found"),
            Self::AlreadyExists => write!(f, "the id is already in use"),
            Self::Conflict { current } => {
                write!(f, "the reminder is now at revision {current}")
            }
            Self::Archived => write!(f, "the reminder is archived"),
            Self::Invalid(invalid) => write!(f, "invalid request: {invalid:?}"),
            Self::NotResolvable => write!(f, "the slot is too far ahead to resolve"),
            Self::NotLatest => write!(f, "only the latest resolution can be undone"),
            Self::Transition(error) => write!(f, "{error}"),
            Self::Zone(error) => write!(f, "{error}"),
            Self::MissingProgress => write!(f, "the reminder has no progress"),
            Self::Storage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<Invalid> for CommandError {
    fn from(invalid: Invalid) -> Self {
        Self::Invalid(invalid)
    }
}

impl From<TransitionError> for CommandError {
    fn from(error: TransitionError) -> Self {
        Self::Transition(error)
    }
}

impl From<ZoneError> for CommandError {
    fn from(error: ZoneError) -> Self {
        Self::Zone(error)
    }
}

impl From<StorageError> for CommandError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::AlreadyExists => Self::AlreadyExists,
            StorageError::Missing => Self::NotFound,
            other => Self::Storage(other),
        }
    }
}

pub trait Reminders {
    fn create(
        &mut self,
        ctx: &Context,
        id: ReminderId,
        draft: ReminderDraft,
    ) -> Result<ReminderState, CommandError>;

    /// Applies `edit` if the reminder is still at `base_revision`.
    fn edit(
        &mut self,
        ctx: &Context,
        id: ReminderId,
        base_revision: u64,
        edit: ReminderEdit,
    ) -> Result<ReminderState, CommandError>;

    /// Tombstones the reminder and drops its progress. Its resolutions stay.
    fn delete(
        &mut self,
        ctx: &Context,
        id: ReminderId,
        base_revision: u64,
    ) -> Result<(), CommandError>;

    /// Records a response to the reminder.
    ///
    /// | Plan | Outcome | Effect |
    /// |---|---|---|
    /// | one-off | `Done` with no slot | completes it |
    /// | cadence | `Done` or `Skipped` with a slot | moves the watermark |
    /// | cadence | `Done` with no slot | log only |
    /// | tracker | `Done` with no slot | log only |
    ///
    /// A slot may be any past slot or the next pending one.
    fn resolve(
        &mut self,
        ctx: &Context,
        id: ReminderId,
        resolution_id: ResolutionId,
        outcome: Outcome,
        note: Option<String>,
    ) -> Result<Resolved, CommandError>;

    /// Undoes a resolution. The latest one that moved progress rolls progress
    /// back. A log-only resolution can be undone at any time. Either way it is
    /// retracted from the archive.
    fn undo(
        &mut self,
        ctx: &Context,
        id: ReminderId,
        resolution_id: ResolutionId,
    ) -> Result<(), CommandError>;

    fn get(&self, id: ReminderId) -> Result<ReminderState, CommandError>;

    /// Every reminder that is not deleted, oldest first.
    fn list(&self) -> Result<Vec<ReminderState>, CommandError>;

    /// Pending slots of reminders that are neither archived nor deleted, ordered
    /// by when they fire, at most `limit`.
    fn upcoming(&self, ctx: &Context, limit: usize) -> Result<UpcomingList, CommandError>;

    /// A reminder's resolutions, newest first, without retracted ones.
    fn history(&self, id: ReminderId) -> Result<Vec<Resolution>, CommandError>;
}
