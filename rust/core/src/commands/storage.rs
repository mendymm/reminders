//! The persistence port the command layer works against. SQLite implements it
//! (see `store`). Writes go in as one [`WriteSet`] that must apply atomically,
//! with the revisions each write expects checked.

use std::fmt;

use crate::model::{Progress, Reminder, ReminderId, Resolution, ResolutionId};

/// Why storage refused or failed an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// A row that must not exist already does.
    AlreadyExists,
    /// A row to update does not exist.
    Missing,
    /// A row changed since it was read.
    StaleRevision,
    /// Stored data could not be turned back into a valid model value.
    Corrupt(String),
    /// The backend failed.
    Backend(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists => write!(f, "the row already exists"),
            Self::Missing => write!(f, "the row does not exist"),
            Self::StaleRevision => write!(f, "the row changed since it was read"),
            Self::Corrupt(message) => write!(f, "stored data is invalid: {message}"),
            Self::Backend(message) => write!(f, "storage failed: {message}"),
        }
    }
}

impl std::error::Error for StorageError {}

/// What a write expects to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// No row with this key.
    Absent,
    /// A row at exactly this revision.
    Revision(u64),
}

/// Checks a row's current revision, `None` when there is no row, against what a
/// write expects. Every [`Storage`] applies this rule.
pub fn check_expect(current: Option<u64>, expect: Expect) -> Result<(), StorageError> {
    match (current, expect) {
        (None, Expect::Absent) => Ok(()),
        (Some(_), Expect::Absent) => Err(StorageError::AlreadyExists),
        (Some(revision), Expect::Revision(expected)) if revision == expected => Ok(()),
        _ => Err(StorageError::StaleRevision),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressWrite {
    Put(Progress, Expect),
    Delete(ReminderId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionWrite {
    /// Add a resolution. Its id must not exist.
    Insert(Resolution),
    /// Replace an existing resolution, such as to tombstone it.
    Update(Resolution),
}

/// Writes that must apply together or not at all.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WriteSet {
    pub reminder: Option<(Reminder, Expect)>,
    pub progress: Option<ProgressWrite>,
    pub resolution: Option<ResolutionWrite>,
}

pub trait Storage {
    /// A reminder by id, including a deleted one.
    fn reminder(&self, id: ReminderId) -> Result<Option<Reminder>, StorageError>;
    /// Every reminder, including deleted ones.
    fn reminders(&self) -> Result<Vec<Reminder>, StorageError>;
    fn progress(&self, id: ReminderId) -> Result<Option<Progress>, StorageError>;
    fn resolution(&self, id: ResolutionId) -> Result<Option<Resolution>, StorageError>;
    /// Every resolution of a reminder, retracted ones included, newest first.
    fn resolutions(&self, id: ReminderId) -> Result<Vec<Resolution>, StorageError>;
    /// Applies all of `writes` atomically, or none of them.
    fn commit(&mut self, writes: WriteSet) -> Result<(), StorageError>;
}
