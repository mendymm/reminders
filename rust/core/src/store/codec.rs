//! Rows to and from model values. Reading goes through two steps: a row becomes a
//! plain `Raw*` struct, then `decode` turns that into a model value, running the
//! model's validation. Stored data that fails it is reported as
//! [`StorageError::Corrupt`], never trusted.
//!
//! Timestamps are stored as Unix milliseconds, so anything finer is dropped.

use std::fmt::Display;

use jiff::{Timestamp, civil};
use rusqlite::Row;
use uuid::Uuid;

use crate::commands::StorageError;
use crate::model::{
    Meta, Outcome, Progress, ProgressState, Reminder, ReminderId, Resolution, ResolutionId, Slot,
    Title, Watermark,
};

fn corrupt(what: impl Display) -> StorageError {
    StorageError::Corrupt(what.to_string())
}

pub(super) fn id_bytes(id: Uuid) -> Vec<u8> {
    id.as_bytes().to_vec()
}

fn uuid_from_sql(bytes: &[u8]) -> Result<Uuid, StorageError> {
    Uuid::from_slice(bytes).map_err(corrupt)
}

fn timestamp_from_sql(milliseconds: i64) -> Result<Timestamp, StorageError> {
    Timestamp::from_millisecond(milliseconds).map_err(corrupt)
}

fn optional_timestamp_from_sql(
    milliseconds: Option<i64>,
) -> Result<Option<Timestamp>, StorageError> {
    milliseconds.map(timestamp_from_sql).transpose()
}

fn revision_to_sql(revision: u64) -> Result<i64, StorageError> {
    i64::try_from(revision)
        .map_err(|_| StorageError::Backend("the revision does not fit in the database".to_owned()))
}

fn revision_from_sql(revision: i64) -> Result<u64, StorageError> {
    u64::try_from(revision).map_err(corrupt)
}

fn slot_to_sql(slot: Slot) -> String {
    slot.0.to_string()
}

fn slot_from_sql(text: &str) -> Result<Slot, StorageError> {
    text.parse::<civil::DateTime>().map(Slot).map_err(corrupt)
}

fn optional_slot_from_sql(text: Option<String>) -> Result<Option<Slot>, StorageError> {
    text.as_deref().map(slot_from_sql).transpose()
}

pub(super) struct RawReminder {
    pub(super) id: Vec<u8>,
    pub(super) title: String,
    pub(super) notes: String,
    pub(super) plan: String,
    pub(super) archived_at: Option<i64>,
    pub(super) revision: i64,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
    pub(super) deleted_at: Option<i64>,
}

impl RawReminder {
    pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            title: row.get(1)?,
            notes: row.get(2)?,
            plan: row.get(3)?,
            archived_at: row.get(4)?,
            revision: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            deleted_at: row.get(8)?,
        })
    }

    pub(super) fn encode(reminder: &Reminder) -> Result<Self, StorageError> {
        Ok(Self {
            id: id_bytes(reminder.id.0),
            title: reminder.title.as_str().to_owned(),
            notes: reminder.notes.clone(),
            plan: serde_json::to_string(&reminder.plan)
                .map_err(|error| StorageError::Backend(error.to_string()))?,
            archived_at: reminder.archived_at.map(Timestamp::as_millisecond),
            revision: revision_to_sql(reminder.meta.revision)?,
            created_at: reminder.meta.created_at.as_millisecond(),
            updated_at: reminder.meta.updated_at.as_millisecond(),
            deleted_at: reminder.meta.deleted_at.map(Timestamp::as_millisecond),
        })
    }

    pub(super) fn decode(self) -> Result<Reminder, StorageError> {
        Ok(Reminder {
            id: ReminderId(uuid_from_sql(&self.id)?),
            title: Title::new(&self.title).map_err(corrupt)?,
            notes: self.notes,
            plan: serde_json::from_str(&self.plan).map_err(corrupt)?,
            archived_at: optional_timestamp_from_sql(self.archived_at)?,
            meta: Meta {
                revision: revision_from_sql(self.revision)?,
                created_at: timestamp_from_sql(self.created_at)?,
                updated_at: timestamp_from_sql(self.updated_at)?,
                deleted_at: optional_timestamp_from_sql(self.deleted_at)?,
            },
        })
    }
}

pub(super) struct RawProgress {
    pub(super) reminder_id: Vec<u8>,
    pub(super) kind: String,
    pub(super) done_at: Option<i64>,
    pub(super) through: Option<String>,
    pub(super) handled: Option<i64>,
    pub(super) previous_through: Option<String>,
    pub(super) previous_handled: Option<i64>,
    pub(super) last_resolution: Option<Vec<u8>>,
    pub(super) revision: i64,
    pub(super) updated_at: i64,
}

impl RawProgress {
    pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            reminder_id: row.get(0)?,
            kind: row.get(1)?,
            done_at: row.get(2)?,
            through: row.get(3)?,
            handled: row.get(4)?,
            previous_through: row.get(5)?,
            previous_handled: row.get(6)?,
            last_resolution: row.get(7)?,
            revision: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }

    pub(super) fn encode(progress: &Progress) -> Result<Self, StorageError> {
        let mut raw = Self {
            reminder_id: id_bytes(progress.reminder_id.0),
            kind: String::new(),
            done_at: None,
            through: None,
            handled: None,
            previous_through: None,
            previous_handled: None,
            last_resolution: progress.last_resolution.map(|id| id_bytes(id.0)),
            revision: revision_to_sql(progress.revision)?,
            updated_at: progress.updated_at.as_millisecond(),
        };
        match &progress.state {
            ProgressState::Once { done_at } => {
                "once".clone_into(&mut raw.kind);
                raw.done_at = done_at.map(Timestamp::as_millisecond);
            }
            ProgressState::Cadence { current, previous } => {
                "cadence".clone_into(&mut raw.kind);
                raw.through = current.through.map(slot_to_sql);
                raw.handled = Some(i64::from(current.handled));
                if let Some(previous) = previous {
                    raw.previous_through = previous.through.map(slot_to_sql);
                    raw.previous_handled = Some(i64::from(previous.handled));
                }
            }
        }
        Ok(raw)
    }

    pub(super) fn decode(self) -> Result<Progress, StorageError> {
        let state = match self.kind.as_str() {
            "once" => ProgressState::Once {
                done_at: optional_timestamp_from_sql(self.done_at)?,
            },
            "cadence" => {
                let handled = self
                    .handled
                    .ok_or_else(|| corrupt("a cadence progress row has no handled count"))?;
                let previous = match self.previous_handled {
                    None => None,
                    Some(handled) => Some(Watermark {
                        through: optional_slot_from_sql(self.previous_through)?,
                        handled: u32::try_from(handled).map_err(corrupt)?,
                    }),
                };
                ProgressState::Cadence {
                    current: Watermark {
                        through: optional_slot_from_sql(self.through)?,
                        handled: u32::try_from(handled).map_err(corrupt)?,
                    },
                    previous,
                }
            }
            other => return Err(corrupt(format!("unknown progress kind {other:?}"))),
        };
        Ok(Progress {
            reminder_id: ReminderId(uuid_from_sql(&self.reminder_id)?),
            state,
            last_resolution: self
                .last_resolution
                .as_deref()
                .map(uuid_from_sql)
                .transpose()?
                .map(ResolutionId),
            revision: revision_from_sql(self.revision)?,
            updated_at: timestamp_from_sql(self.updated_at)?,
        })
    }
}

pub(super) struct RawResolution {
    pub(super) id: Vec<u8>,
    pub(super) reminder_id: Vec<u8>,
    pub(super) resolved_at: i64,
    pub(super) outcome: String,
    pub(super) slot: Option<String>,
    pub(super) note: Option<String>,
    pub(super) deleted_at: Option<i64>,
}

impl RawResolution {
    pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            reminder_id: row.get(1)?,
            resolved_at: row.get(2)?,
            outcome: row.get(3)?,
            slot: row.get(4)?,
            note: row.get(5)?,
            deleted_at: row.get(6)?,
        })
    }

    pub(super) fn encode(resolution: &Resolution) -> Self {
        let (outcome, slot) = match resolution.outcome {
            Outcome::Done { slot } => ("done", slot.map(slot_to_sql)),
            Outcome::Skipped { slot } => ("skipped", Some(slot_to_sql(slot))),
        };
        Self {
            id: id_bytes(resolution.id.0),
            reminder_id: id_bytes(resolution.reminder_id.0),
            resolved_at: resolution.resolved_at.as_millisecond(),
            outcome: outcome.to_owned(),
            slot,
            note: resolution.note.clone(),
            deleted_at: resolution.deleted_at.map(Timestamp::as_millisecond),
        }
    }

    pub(super) fn decode(self) -> Result<Resolution, StorageError> {
        let outcome = match self.outcome.as_str() {
            "done" => Outcome::Done {
                slot: optional_slot_from_sql(self.slot)?,
            },
            "skipped" => Outcome::Skipped {
                slot: self
                    .slot
                    .as_deref()
                    .map(slot_from_sql)
                    .transpose()?
                    .ok_or_else(|| corrupt("a skipped resolution has no slot"))?,
            },
            other => return Err(corrupt(format!("unknown outcome {other:?}"))),
        };
        Ok(Resolution {
            id: ResolutionId(uuid_from_sql(&self.id)?),
            reminder_id: ReminderId(uuid_from_sql(&self.reminder_id)?),
            resolved_at: timestamp_from_sql(self.resolved_at)?,
            outcome,
            note: self.note,
            deleted_at: optional_timestamp_from_sql(self.deleted_at)?,
        })
    }
}
