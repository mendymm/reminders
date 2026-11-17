//! Pure transitions on [`Progress`]. No I/O and no clock: callers pass `now`.
//!
//! Resolutions are archival, so none of these take an [`crate::model::Outcome`].
//! Done and skipped advance progress identically.

use std::fmt;

use jiff::Timestamp;

use crate::model::{Plan, Progress, ProgressState, ReminderId, Recurrence, Slot, Watermark};
use crate::recurrence::next_slot;

#[cfg(test)]
mod tests;

/// Why a transition was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// The progress state does not match the reminder's plan.
    WrongPlan,
    /// A one-off reminder is already completed.
    AlreadyDone,
    /// The slot is missing, or is not a slot of the recurrence.
    NotASlot,
    /// The slot is at or before the watermark.
    AlreadyHandled,
    /// There is nothing to undo.
    NothingToUndo,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongPlan => write!(f, "progress does not match the reminder's plan"),
            Self::AlreadyDone => write!(f, "the reminder is already completed"),
            Self::NotASlot => write!(f, "not a slot of the recurrence"),
            Self::AlreadyHandled => write!(f, "the slot is already handled"),
            Self::NothingToUndo => write!(f, "nothing to undo"),
        }
    }
}

impl std::error::Error for TransitionError {}

impl Progress {
    /// Progress for a newly created reminder. A tracker has none.
    pub fn initial(reminder_id: ReminderId, plan: &Plan, now: Timestamp) -> Option<Self> {
        let state = match plan {
            Plan::Once { .. } => ProgressState::Once { done_at: None },
            Plan::Ongoing { cadence: Some(_) } => ProgressState::Cadence {
                current: Watermark {
                    through: None,
                    handled: 0,
                },
                previous: None,
            },
            Plan::Ongoing { cadence: None } => return None,
        };
        Some(Self {
            reminder_id,
            state,
            revision: 1,
            updated_at: now,
        })
    }

    /// Records a resolution. A one-off reminder is completed and `slot` is
    /// ignored. For a cadence, `slot` must be a later slot than the watermark;
    /// the watermark moves to it, closing any slots in between.
    pub fn resolve(
        &self,
        plan: &Plan,
        slot: Option<Slot>,
        now: Timestamp,
    ) -> Result<Self, TransitionError> {
        let state = match (&self.state, plan) {
            (ProgressState::Once { done_at }, Plan::Once { .. }) => {
                if done_at.is_some() {
                    return Err(TransitionError::AlreadyDone);
                }
                ProgressState::Once {
                    done_at: Some(now),
                }
            }
            (
                ProgressState::Cadence { current, .. },
                Plan::Ongoing {
                    cadence: Some(cadence),
                },
            ) => {
                let slot = slot.ok_or(TransitionError::NotASlot)?;
                ProgressState::Cadence {
                    current: advance(&cadence.recurrence, *current, slot)?,
                    previous: Some(*current),
                }
            }
            _ => return Err(TransitionError::WrongPlan),
        };
        Ok(self.with_state(state, now))
    }

    /// Undoes the last resolution. For a cadence this is one level deep.
    pub fn undo(&self, now: Timestamp) -> Result<Self, TransitionError> {
        let state = match &self.state {
            ProgressState::Once { done_at: Some(_) } => ProgressState::Once { done_at: None },
            ProgressState::Cadence {
                previous: Some(previous),
                ..
            } => ProgressState::Cadence {
                current: *previous,
                previous: None,
            },
            ProgressState::Once { done_at: None }
            | ProgressState::Cadence { previous: None, .. } => {
                return Err(TransitionError::NothingToUndo);
            }
        };
        Ok(self.with_state(state, now))
    }

    /// The slot waiting to be resolved: a one-off reminder's due time while it is
    /// open, or the first slot after a cadence's watermark.
    pub fn next_pending(&self, plan: &Plan) -> Option<Slot> {
        match (&self.state, plan) {
            (ProgressState::Once { done_at: None }, Plan::Once { due: Some(due) }) => {
                Some(Slot(due.when.local))
            }
            (
                ProgressState::Cadence { current, .. },
                Plan::Ongoing {
                    cadence: Some(cadence),
                },
            ) => next_slot(&cadence.recurrence, *current),
            _ => None,
        }
    }

    fn with_state(&self, state: ProgressState, now: Timestamp) -> Self {
        Self {
            reminder_id: self.reminder_id,
            state,
            revision: self.revision.saturating_add(1),
            updated_at: now,
        }
    }
}

/// Moves the watermark forward to `slot`, counting every slot passed over.
fn advance(
    recurrence: &Recurrence,
    current: Watermark,
    slot: Slot,
) -> Result<Watermark, TransitionError> {
    if current.through.is_some_and(|through| slot <= through) {
        return Err(TransitionError::AlreadyHandled);
    }
    let mut mark = current;
    loop {
        let next = next_slot(recurrence, mark).ok_or(TransitionError::NotASlot)?;
        mark = Watermark {
            through: Some(next),
            handled: mark.handled.saturating_add(1),
        };
        match next.cmp(&slot) {
            std::cmp::Ordering::Equal => return Ok(mark),
            std::cmp::Ordering::Greater => return Err(TransitionError::NotASlot),
            std::cmp::Ordering::Less => {}
        }
    }
}
