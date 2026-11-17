//! The error every exported call can return.

use std::fmt;

use crate::commands::{CommandError, Invalid};
use crate::instant::ZoneError;
use crate::progress::TransitionError;

/// Why an exported call failed. Kotlin sees one exception type with a subclass
/// per case.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum ReminderError {
    /// An argument could not be turned into a valid value: a malformed id, date
    /// or time, a blank title, a zero count, an unknown device zone.
    InvalidInput { reason: String },
    NotFound,
    AlreadyExists,
    /// The reminder changed since the revision the edit was based on.
    Conflict { current: u64 },
    Archived,
    /// A fixed zone name the time zone database does not know.
    UnknownZone { name: String },
    EndsBeforeStart,
    OutcomeNotAllowed,
    NotResolvable,
    NotLatest,
    WrongPlan,
    AlreadyDone,
    NotASlot,
    AlreadyHandled,
    NothingToUndo,
    ZoneOutOfRange,
    MissingProgress,
    Storage { reason: String },
}

impl fmt::Display for ReminderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { reason } => write!(f, "invalid input: {reason}"),
            Self::NotFound => write!(f, "not found"),
            Self::AlreadyExists => write!(f, "the id is already in use"),
            Self::Conflict { current } => write!(f, "the reminder is now at revision {current}"),
            Self::Archived => write!(f, "the reminder is archived"),
            Self::UnknownZone { name } => write!(f, "unknown time zone: {name}"),
            Self::EndsBeforeStart => write!(f, "the recurrence ends before it starts"),
            Self::OutcomeNotAllowed => write!(f, "the plan cannot take that outcome"),
            Self::NotResolvable => write!(f, "the slot is too far ahead to resolve"),
            Self::NotLatest => write!(f, "only the latest resolution can be undone"),
            Self::WrongPlan => write!(f, "progress does not match the reminder's plan"),
            Self::AlreadyDone => write!(f, "the reminder is already completed"),
            Self::NotASlot => write!(f, "not a slot of the recurrence"),
            Self::AlreadyHandled => write!(f, "the slot is already handled"),
            Self::NothingToUndo => write!(f, "nothing to undo"),
            Self::ZoneOutOfRange => write!(f, "the instant is out of range"),
            Self::MissingProgress => write!(f, "the reminder has no progress"),
            Self::Storage { reason } => write!(f, "storage failed: {reason}"),
        }
    }
}

impl std::error::Error for ReminderError {}

impl From<CommandError> for ReminderError {
    fn from(error: CommandError) -> Self {
        match error {
            CommandError::NotFound => Self::NotFound,
            CommandError::AlreadyExists => Self::AlreadyExists,
            CommandError::Conflict { current } => Self::Conflict { current },
            CommandError::Archived => Self::Archived,
            CommandError::Invalid(Invalid::UnknownZone(name)) => Self::UnknownZone { name },
            CommandError::Invalid(Invalid::EndsBeforeStart) => Self::EndsBeforeStart,
            CommandError::Invalid(Invalid::OutcomeNotAllowed) => Self::OutcomeNotAllowed,
            CommandError::NotResolvable => Self::NotResolvable,
            CommandError::NotLatest => Self::NotLatest,
            CommandError::Transition(TransitionError::WrongPlan) => Self::WrongPlan,
            CommandError::Transition(TransitionError::AlreadyDone) => Self::AlreadyDone,
            CommandError::Transition(TransitionError::NotASlot) => Self::NotASlot,
            CommandError::Transition(TransitionError::AlreadyHandled) => Self::AlreadyHandled,
            CommandError::Transition(TransitionError::NothingToUndo) => Self::NothingToUndo,
            CommandError::Zone(ZoneError::UnknownZone(name)) => Self::UnknownZone { name },
            CommandError::Zone(ZoneError::OutOfRange) => Self::ZoneOutOfRange,
            CommandError::MissingProgress => Self::MissingProgress,
            CommandError::Storage(error) => Self::Storage {
                reason: error.to_string(),
            },
        }
    }
}
