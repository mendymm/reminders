//! [`LocalReminders`]: the command layer over a [`Storage`].

use jiff::Timestamp;

use super::validate::validate_plan;
use super::{
    CommandError, Context, Invalid, ReminderDraft, ReminderEdit, ReminderState, Reminders,
    Resolved, Unresolved, UnresolvedReason, Upcoming, UpcomingList,
};
use super::storage::{Expect, ProgressWrite, ResolutionWrite, Storage, WriteSet};
use crate::instant::{ZoneError, instant_of, zone_of};
use crate::model::{
    Alert, Cadence, End, Meta, Outcome, Plan, Progress, ProgressState, Reminder, ReminderId,
    Resolution, ResolutionId, Slot, Watermark, Zone,
};
use crate::recurrence::next_slot;

pub struct LocalReminders<S: Storage> {
    pub(super) storage: S,
}

impl<S: Storage> LocalReminders<S> {
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    /// The reminder, unless it is missing or deleted.
    fn live(&self, id: ReminderId) -> Result<Reminder, CommandError> {
        match self.storage.reminder(id)? {
            Some(reminder) if reminder.meta.deleted_at.is_none() => Ok(reminder),
            _ => Err(CommandError::NotFound),
        }
    }

    fn state_of(&self, reminder: Reminder) -> Result<ReminderState, CommandError> {
        let progress = self.storage.progress(reminder.id)?;
        Ok(ReminderState { reminder, progress })
    }

    /// For each reminder that is neither archived nor deleted, the slot `next`
    /// picks, ordered by when it fires, at most `limit`. A reminder `next` cannot
    /// resolve is reported, not allowed to fail the whole call.
    fn schedule_list(
        &self,
        limit: usize,
        next: impl Fn(&Plan, &Progress) -> Result<Option<(Slot, Timestamp)>, ZoneError>,
    ) -> Result<UpcomingList, CommandError> {
        let mut upcoming = Vec::new();
        let mut unresolved = Vec::new();

        for reminder in self.storage.reminders()? {
            if reminder.meta.deleted_at.is_some() || reminder.archived_at.is_some() {
                continue;
            }
            let Some((_, alert)) = scheduled(&reminder.plan) else {
                continue;
            };
            let Some(progress) = self.storage.progress(reminder.id)? else {
                unresolved.push(Unresolved {
                    reminder_id: reminder.id,
                    reason: UnresolvedReason::MissingProgress,
                });
                continue;
            };
            match next(&reminder.plan, &progress) {
                Ok(Some((slot, at))) => upcoming.push(Upcoming {
                    reminder_id: reminder.id,
                    title: reminder.title.clone(),
                    slot,
                    at,
                    alert: alert.clone(),
                }),
                Ok(None) => {}
                Err(error) => unresolved.push(Unresolved {
                    reminder_id: reminder.id,
                    reason: UnresolvedReason::Zone(error),
                }),
            }
        }

        upcoming.sort_by_key(|item| (item.at, item.reminder_id.0));
        upcoming.truncate(limit);
        unresolved.sort_by_key(|item| item.reminder_id.0);
        Ok(UpcomingList {
            upcoming,
            unresolved,
        })
    }
}

/// What kind of progress a plan needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Once,
    Cadence,
    Tracker,
}

fn kind(plan: &Plan) -> Kind {
    match plan {
        Plan::Once { .. } => Kind::Once,
        Plan::Ongoing { cadence: Some(_) } => Kind::Cadence,
        Plan::Ongoing { cadence: None } => Kind::Tracker,
    }
}

/// The zone and alert of a plan that fires.
fn scheduled(plan: &Plan) -> Option<(&Zone, &Alert)> {
    match plan {
        Plan::Once { due: Some(due) } => Some((&due.when.zone, &due.alert)),
        Plan::Ongoing {
            cadence: Some(cadence),
        } => Some((&cadence.recurrence.zone, &cadence.alert)),
        Plan::Once { due: None } | Plan::Ongoing { cadence: None } => None,
    }
}

/// A slot may be resolved when it is the next pending one, even if it has not
/// fired yet, or when it has already fired.
fn check_resolvable(
    ctx: &Context,
    plan: &Plan,
    cadence: &Cadence,
    progress: &Progress,
    slot: Slot,
) -> Result<(), CommandError> {
    if progress.next_pending(plan) == Some(slot) {
        return Ok(());
    }
    let at = instant_of(slot, &cadence.recurrence.zone, &ctx.device_zone)?;
    if at <= ctx.now {
        Ok(())
    } else {
        Err(CommandError::NotResolvable)
    }
}

impl<S: Storage> Reminders for LocalReminders<S> {
    fn create(
        &mut self,
        ctx: &Context,
        id: ReminderId,
        draft: ReminderDraft,
    ) -> Result<ReminderState, CommandError> {
        validate_plan(&draft.plan)?;
        let reminder = Reminder {
            id,
            title: draft.title,
            notes: draft.notes,
            plan: draft.plan,
            archived_at: None,
            meta: Meta {
                revision: 1,
                created_at: ctx.now,
                updated_at: ctx.now,
                deleted_at: None,
            },
        };
        let progress = Progress::initial(id, &reminder.plan, ctx.now);
        self.storage.commit(WriteSet {
            reminder: Some((reminder.clone(), Expect::Absent)),
            progress: progress
                .clone()
                .map(|progress| ProgressWrite::Put(progress, Expect::Absent)),
            resolution: None,
        })?;
        Ok(ReminderState { reminder, progress })
    }

    fn edit(
        &mut self,
        ctx: &Context,
        id: ReminderId,
        base_revision: u64,
        edit: ReminderEdit,
    ) -> Result<ReminderState, CommandError> {
        let current = self.live(id)?;
        if current.meta.revision != base_revision {
            return Err(CommandError::Conflict {
                current: current.meta.revision,
            });
        }
        if current.archived_at.is_some() && edit.archived != Some(false) {
            return Err(CommandError::Archived);
        }

        let mut next = current.clone();
        if let Some(title) = edit.title {
            next.title = title;
        }
        if let Some(notes) = edit.notes {
            next.notes = notes;
        }
        if let Some(plan) = edit.plan {
            validate_plan(&plan)?;
            next.plan = plan;
        }
        match edit.archived {
            Some(true) if next.archived_at.is_none() => next.archived_at = Some(ctx.now),
            Some(false) => next.archived_at = None,
            Some(true) | None => {}
        }
        next.meta.revision = next.meta.revision.saturating_add(1);
        next.meta.updated_at = ctx.now;

        let old_progress = self.storage.progress(id)?;
        let progress_write = if kind(&current.plan) == kind(&next.plan) {
            None
        } else {
            match (Progress::initial(id, &next.plan, ctx.now), &old_progress) {
                (Some(new), Some(old)) => Some(ProgressWrite::Put(
                    Progress {
                        revision: old.revision.saturating_add(1),
                        ..new
                    },
                    Expect::Revision(old.revision),
                )),
                (Some(new), None) => Some(ProgressWrite::Put(new, Expect::Absent)),
                (None, Some(_)) => Some(ProgressWrite::Delete(id)),
                (None, None) => None,
            }
        };
        let progress = match &progress_write {
            Some(ProgressWrite::Put(new, _)) => Some(new.clone()),
            Some(ProgressWrite::Delete(_)) => None,
            None => old_progress,
        };

        self.storage.commit(WriteSet {
            reminder: Some((next.clone(), Expect::Revision(current.meta.revision))),
            progress: progress_write,
            resolution: None,
        })?;
        Ok(ReminderState {
            reminder: next,
            progress,
        })
    }

    fn delete(
        &mut self,
        ctx: &Context,
        id: ReminderId,
        base_revision: u64,
    ) -> Result<(), CommandError> {
        let current = self.live(id)?;
        if current.meta.revision != base_revision {
            return Err(CommandError::Conflict {
                current: current.meta.revision,
            });
        }
        let mut next = current.clone();
        next.meta.deleted_at = Some(ctx.now);
        next.meta.revision = next.meta.revision.saturating_add(1);
        next.meta.updated_at = ctx.now;

        let has_progress = self.storage.progress(id)?.is_some();
        self.storage.commit(WriteSet {
            reminder: Some((next, Expect::Revision(current.meta.revision))),
            progress: has_progress.then_some(ProgressWrite::Delete(id)),
            resolution: None,
        })?;
        Ok(())
    }

    fn resolve(
        &mut self,
        ctx: &Context,
        id: ReminderId,
        resolution_id: ResolutionId,
        outcome: Outcome,
        note: Option<String>,
    ) -> Result<Resolved, CommandError> {
        let reminder = self.live(id)?;
        if reminder.archived_at.is_some() {
            return Err(CommandError::Archived);
        }
        let progress = self.storage.progress(id)?;

        let advanced = match (&reminder.plan, &outcome) {
            (Plan::Once { .. }, Outcome::Done { slot: None }) => {
                let progress = progress.as_ref().ok_or(CommandError::MissingProgress)?;
                Some(progress.resolve(&reminder.plan, None, resolution_id, ctx.now)?)
            }
            (
                Plan::Ongoing {
                    cadence: Some(cadence),
                },
                Outcome::Done { slot: Some(slot) } | Outcome::Skipped { slot },
            ) => {
                let progress = progress.as_ref().ok_or(CommandError::MissingProgress)?;
                check_resolvable(ctx, &reminder.plan, cadence, progress, *slot)?;
                Some(progress.resolve(&reminder.plan, Some(*slot), resolution_id, ctx.now)?)
            }
            (Plan::Ongoing { .. }, Outcome::Done { slot: None }) => None,
            _ => return Err(Invalid::OutcomeNotAllowed.into()),
        };

        let resolution = Resolution {
            id: resolution_id,
            reminder_id: id,
            resolved_at: ctx.now,
            outcome,
            note,
            deleted_at: None,
        };
        let progress_write = match (&advanced, &progress) {
            (Some(new), Some(old)) => Some(ProgressWrite::Put(
                new.clone(),
                Expect::Revision(old.revision),
            )),
            _ => None,
        };
        self.storage.commit(WriteSet {
            reminder: None,
            progress: progress_write,
            resolution: Some(ResolutionWrite::Insert(resolution.clone())),
        })?;
        Ok(Resolved {
            resolution,
            progress: advanced.or(progress),
        })
    }

    fn undo(
        &mut self,
        ctx: &Context,
        id: ReminderId,
        resolution_id: ResolutionId,
    ) -> Result<(), CommandError> {
        let reminder = self.live(id)?;
        if reminder.archived_at.is_some() {
            return Err(CommandError::Archived);
        }
        let progress = self.storage.progress(id)?;
        let mut resolution = self
            .storage
            .resolution(resolution_id)?
            .ok_or(CommandError::NotFound)?;
        if resolution.reminder_id != id || resolution.deleted_at.is_some() {
            return Err(CommandError::NotFound);
        }

        let progress_write = match &progress {
            Some(progress) if progress.last_resolution == Some(resolution_id) => {
                Some(ProgressWrite::Put(
                    progress.undo(ctx.now)?,
                    Expect::Revision(progress.revision),
                ))
            }
            _ => match resolution.outcome {
                // Logged without moving progress, so nothing to roll back.
                Outcome::Done { slot: None } => None,
                Outcome::Done { slot: Some(_) } | Outcome::Skipped { .. } => {
                    return Err(CommandError::NotLatest);
                }
            },
        };

        resolution.deleted_at = Some(ctx.now);
        self.storage.commit(WriteSet {
            reminder: None,
            progress: progress_write,
            resolution: Some(ResolutionWrite::Update(resolution)),
        })?;
        Ok(())
    }

    fn get(&self, id: ReminderId) -> Result<ReminderState, CommandError> {
        let reminder = self.live(id)?;
        self.state_of(reminder)
    }

    fn list(&self) -> Result<Vec<ReminderState>, CommandError> {
        let mut reminders: Vec<Reminder> = self
            .storage
            .reminders()?
            .into_iter()
            .filter(|reminder| reminder.meta.deleted_at.is_none())
            .collect();
        reminders.sort_by_key(|reminder| (reminder.meta.created_at, reminder.id.0));
        reminders
            .into_iter()
            .map(|reminder| self.state_of(reminder))
            .collect()
    }

    fn upcoming(&self, ctx: &Context, limit: usize) -> Result<UpcomingList, CommandError> {
        self.schedule_list(limit, |plan, progress| {
            let Some((zone, _)) = scheduled(plan) else {
                return Ok(None);
            };
            let Some(slot) = progress.next_pending(plan) else {
                return Ok(None);
            };
            Ok(Some((slot, instant_of(slot, zone, &ctx.device_zone)?)))
        })
    }

    fn alarms(&self, ctx: &Context, limit: usize) -> Result<UpcomingList, CommandError> {
        self.schedule_list(limit, |plan, progress| next_alarm(plan, progress, ctx))
    }

    fn history(&self, id: ReminderId) -> Result<Vec<Resolution>, CommandError> {
        self.live(id)?;
        Ok(self
            .storage
            .resolutions(id)?
            .into_iter()
            .filter(|resolution| resolution.deleted_at.is_none())
            .collect())
    }
}

/// The most already-passed slots `next_alarm` steps over before giving up. It
/// only matters for a recurrence that ends after a count, or one that has slots
/// every few minutes and was not looked at for a long time.
const MAX_PASSED_SLOTS: usize = 10_000;

/// The first slot of a plan that fires after `ctx.now`, with when it fires. Slots
/// that already passed unresolved are stepped over, not reported.
fn next_alarm(
    plan: &Plan,
    progress: &Progress,
    ctx: &Context,
) -> Result<Option<(Slot, Timestamp)>, ZoneError> {
    match (&progress.state, plan) {
        (ProgressState::Once { done_at: None }, Plan::Once { due: Some(due) }) => {
            let slot = Slot(due.when.local);
            let at = instant_of(slot, &due.when.zone, &ctx.device_zone)?;
            Ok((at > ctx.now).then_some((slot, at)))
        }
        (
            ProgressState::Cadence { current, .. },
            Plan::Ongoing {
                cadence: Some(cadence),
            },
        ) => {
            let recurrence = &cadence.recurrence;
            let mut mark = *current;
            // Jump to now instead of stepping through every slot since the
            // watermark. A recurrence that ends after a count needs the count of
            // slots passed, so it steps.
            if !matches!(recurrence.end, Some(End::Count(_))) {
                let zone = zone_of(&recurrence.zone, &ctx.device_zone)?;
                let local_now = Slot(zone.to_datetime(ctx.now));
                if mark.through.is_none_or(|through| through < local_now) {
                    mark.through = Some(local_now);
                }
            }
            for _ in 0..MAX_PASSED_SLOTS {
                let Some(slot) = next_slot(recurrence, mark) else {
                    return Ok(None);
                };
                let at = instant_of(slot, &recurrence.zone, &ctx.device_zone)?;
                if at > ctx.now {
                    return Ok(Some((slot, at)));
                }
                mark = Watermark {
                    through: Some(slot),
                    handled: mark.handled.saturating_add(1),
                };
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}
