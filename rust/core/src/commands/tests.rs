//! Tests for the command layer, run against an in-memory storage.

use std::collections::HashMap;
use std::num::NonZeroU32;

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan, civil};
use proptest::prelude::*;
use uuid::Uuid;

use super::memory::MemoryStorage;
use super::{
    CommandError, Context, Expect, Invalid, LocalReminders, ProgressWrite, ReminderDraft,
    ReminderEdit, ReminderState, Reminders, Resolved, Storage, Unresolved, UnresolvedReason,
    WriteSet,
};
use crate::instant::ZoneError;
use crate::model::{
    Alert, AlertStyle, Cadence, ClockTimes, Due, End, Meta, Outcome, Plan, Progress,
    ProgressState, Recurrence, Reminder, ReminderId, ResolutionId, Rule, Slot, TimesOfDay, Title,
    Watermark, When, Zone,
};
use crate::progress::TransitionError;
use crate::recurrence::tests::{any_recurrence, oracle};

type Service = LocalReminders<MemoryStorage>;

/// Noon on the 18th. The daily slots of the 16th, 17th and 18th are past, the
/// 19th's is not.
pub(crate) const NOW: &str = "2026-11-18T12:00:00Z";

fn service() -> Service {
    LocalReminders::new(MemoryStorage::default())
}

fn ts(text: &str) -> Timestamp {
    text.parse().expect("a valid timestamp")
}

pub(crate) fn ctx(now: &str) -> Context {
    Context {
        now: ts(now),
        device_zone: TimeZone::UTC,
    }
}

fn ctx_in(now: &str, hours: i8) -> Context {
    Context {
        now: ts(now),
        device_zone: TimeZone::fixed(jiff::tz::offset(hours)),
    }
}

fn rid(n: u128) -> ReminderId {
    ReminderId(Uuid::from_u128(n))
}

fn xid(n: u128) -> ResolutionId {
    ResolutionId(Uuid::from_u128(n))
}

fn title(text: &str) -> Title {
    Title::new(text).expect("a non-blank title")
}

fn alert() -> Alert {
    Alert {
        style: AlertStyle::Notification { sticky: false },
        nudge: None,
    }
}

/// The slot `days` days after the first daily slot, 2026-11-16 at 09:00.
fn day_slot(days: u8) -> Slot {
    Slot((civil::date(2026, 11, 16) + i32::from(days).days()).at(9, 0, 0, 0))
}

fn once_due(local: civil::DateTime, zone: Zone) -> Plan {
    Plan::Once {
        due: Some(Due {
            when: When { local, zone },
            alert: alert(),
        }),
    }
}

fn once_plan() -> Plan {
    once_due(civil::date(2026, 11, 20).at(9, 0, 0, 0), Zone::Floating)
}

/// Daily at `hour`:00 from 2026-11-16.
fn daily(zone: Zone, hour: i8, end: Option<End>) -> Plan {
    Plan::Ongoing {
        cadence: Some(Cadence {
            recurrence: Recurrence {
                starts_on: civil::date(2026, 11, 16),
                zone,
                rule: Rule::Daily {
                    every: NonZeroU32::new(1).expect("one"),
                },
                at: TimesOfDay::At(
                    ClockTimes::new([civil::time(hour, 0, 0, 0)]).expect("one time"),
                ),
                end,
            },
            alert: alert(),
        }),
    }
}

fn daily_plan() -> Plan {
    daily(Zone::Floating, 9, None)
}

fn tracker_plan() -> Plan {
    Plan::Ongoing { cadence: None }
}

fn draft(plan: Plan) -> ReminderDraft {
    ReminderDraft {
        title: title("A reminder"),
        notes: String::new(),
        plan,
    }
}

fn create(s: &mut Service, id: u128, plan: Plan) -> ReminderState {
    s.create(&ctx(NOW), rid(id), draft(plan)).expect("created")
}

fn done(slot: Option<Slot>) -> Outcome {
    Outcome::Done { slot }
}

fn skipped(slot: Slot) -> Outcome {
    Outcome::Skipped { slot }
}

fn resolve(
    s: &mut Service,
    c: &Context,
    id: u128,
    resolution: u128,
    outcome: Outcome,
) -> Result<Resolved, CommandError> {
    s.resolve(c, rid(id), xid(resolution), outcome, None)
}

fn revision(s: &Service, id: u128) -> u64 {
    s.get(rid(id)).expect("exists").reminder.meta.revision
}

fn progress_of(s: &Service, id: u128) -> Option<Progress> {
    s.get(rid(id)).expect("exists").progress
}

fn watermark(progress: &Progress) -> Watermark {
    match &progress.state {
        ProgressState::Cadence { current, .. } => *current,
        ProgressState::Once { .. } => panic!("expected cadence progress"),
    }
}

fn rename(text: &str) -> ReminderEdit {
    ReminderEdit {
        title: Some(title(text)),
        ..ReminderEdit::default()
    }
}

fn replace_plan(plan: Plan) -> ReminderEdit {
    ReminderEdit {
        plan: Some(plan),
        ..ReminderEdit::default()
    }
}

fn archive(on: bool) -> ReminderEdit {
    ReminderEdit {
        archived: Some(on),
        ..ReminderEdit::default()
    }
}

// --- create ---------------------------------------------------------------

#[test]
fn creating_gives_revision_one_and_progress_that_matches_the_plan() {
    let mut s = service();

    let once = create(&mut s, 1, once_plan());
    assert_eq!(once.reminder.meta.revision, 1);
    assert!(matches!(
        once.progress,
        Some(Progress {
            state: ProgressState::Once { done_at: None },
            ..
        })
    ));

    let cadence = create(&mut s, 2, daily_plan());
    assert!(matches!(
        cadence.progress,
        Some(Progress {
            state: ProgressState::Cadence { .. },
            ..
        })
    ));

    let tracker = create(&mut s, 3, tracker_plan());
    assert_eq!(tracker.progress, None);

    assert_eq!(s.get(rid(1)).expect("exists"), once);
}

#[test]
fn creating_with_a_used_id_fails_and_changes_nothing() {
    let mut s = service();
    create(&mut s, 1, tracker_plan());

    let other = ReminderDraft {
        title: title("Something else"),
        notes: String::new(),
        plan: once_plan(),
    };
    assert_eq!(
        s.create(&ctx(NOW), rid(1), other),
        Err(CommandError::AlreadyExists)
    );
    assert_eq!(
        s.get(rid(1)).expect("exists").reminder.title.as_str(),
        "A reminder"
    );
}

#[test]
fn creating_rejects_an_unknown_zone_and_an_end_before_the_start() {
    let mut s = service();
    let c = ctx(NOW);

    let unknown_zone = once_due(
        civil::date(2026, 11, 20).at(9, 0, 0, 0),
        Zone::Fixed("Not/AZone".to_owned()),
    );
    assert_eq!(
        s.create(&c, rid(1), draft(unknown_zone)),
        Err(CommandError::Invalid(Invalid::UnknownZone(
            "Not/AZone".to_owned()
        )))
    );

    let ends_early = daily(
        Zone::Floating,
        9,
        Some(End::Until(civil::date(2026, 11, 15).at(23, 0, 0, 0))),
    );
    assert_eq!(
        s.create(&c, rid(2), draft(ends_early)),
        Err(CommandError::Invalid(Invalid::EndsBeforeStart))
    );

    assert!(s.list().expect("listed").is_empty());
}

// --- edit -----------------------------------------------------------------

#[test]
fn an_edit_needs_the_current_revision() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, tracker_plan());

    assert_eq!(
        s.edit(&c, rid(1), 5, rename("Renamed")),
        Err(CommandError::Conflict { current: 1 })
    );

    let edited = s
        .edit(&c, rid(1), 1, rename("Renamed"))
        .expect("current revision");
    assert_eq!(edited.reminder.meta.revision, 2);

    assert_eq!(
        s.edit(&c, rid(1), 1, rename("Again")),
        Err(CommandError::Conflict { current: 2 })
    );
}

#[test]
fn an_edit_changes_only_the_fields_given() {
    let mut s = service();
    create(&mut s, 1, tracker_plan());
    let later = ctx("2026-11-19T08:00:00Z");

    let edited = s
        .edit(
            &later,
            rid(1),
            1,
            ReminderEdit {
                notes: Some("remember".to_owned()),
                ..ReminderEdit::default()
            },
        )
        .expect("edited");

    assert_eq!(edited.reminder.notes, "remember");
    assert_eq!(edited.reminder.title.as_str(), "A reminder");
    assert_eq!(edited.reminder.meta.updated_at, later.now);
    assert_eq!(edited.reminder.meta.created_at, ts(NOW));
}

#[test]
fn editing_a_cadence_keeps_its_progress() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, daily_plan());
    let resolved = resolve(&mut s, &c, 1, 10, done(Some(day_slot(0)))).expect("resolved");

    s.edit(&c, rid(1), 1, replace_plan(daily(Zone::Floating, 10, None)))
        .expect("edited");

    assert_eq!(progress_of(&s, 1), resolved.progress);
}

#[test]
fn changing_the_kind_of_plan_resets_progress() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, daily_plan());
    resolve(&mut s, &c, 1, 10, done(Some(day_slot(0)))).expect("resolved");

    // Cadence to one-off: fresh progress, with a revision above the old one's.
    let once = s
        .edit(&c, rid(1), revision(&s, 1), replace_plan(once_plan()))
        .expect("edited");
    let progress = once.progress.expect("a one-off has progress");
    assert_eq!(progress.state, ProgressState::Once { done_at: None });
    assert_eq!(progress.last_resolution, None);
    assert_eq!(progress.revision, 3);

    // One-off to tracker: no progress.
    let tracker = s
        .edit(&c, rid(1), revision(&s, 1), replace_plan(tracker_plan()))
        .expect("edited");
    assert_eq!(tracker.progress, None);
    assert_eq!(s.storage.progress(rid(1)).expect("read"), None);

    // Tracker to cadence: progress starts with nothing handled.
    let cadence = s
        .edit(&c, rid(1), revision(&s, 1), replace_plan(daily_plan()))
        .expect("edited");
    let progress = cadence.progress.expect("a cadence has progress");
    assert_eq!(
        watermark(&progress),
        Watermark {
            through: None,
            handled: 0,
        }
    );
}

#[test]
fn a_completed_one_off_stays_completed_when_its_due_time_changes() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, once_plan());
    resolve(&mut s, &c, 1, 10, done(None)).expect("completed");

    let later = once_due(civil::date(2026, 12, 1).at(9, 0, 0, 0), Zone::Floating);
    let edited = s
        .edit(&c, rid(1), 1, replace_plan(later))
        .expect("edited");

    assert!(matches!(
        edited.progress,
        Some(Progress {
            state: ProgressState::Once { done_at: Some(_) },
            ..
        })
    ));
}

#[test]
fn an_archived_reminder_is_frozen_until_it_is_restored() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, once_plan());

    let archived = s.edit(&c, rid(1), 1, archive(true)).expect("archived");
    assert_eq!(archived.reminder.archived_at, Some(c.now));
    assert!(s.upcoming(&c, 10).expect("upcoming").upcoming.is_empty());

    assert_eq!(
        s.edit(&c, rid(1), 2, rename("Nope")),
        Err(CommandError::Archived)
    );
    assert_eq!(
        resolve(&mut s, &c, 1, 10, done(None)).unwrap_err(),
        CommandError::Archived
    );

    let restored = s.edit(&c, rid(1), 2, archive(false)).expect("restored");
    assert_eq!(restored.reminder.archived_at, None);
    resolve(&mut s, &c, 1, 10, done(None)).expect("resolvable again");
}

// --- delete ---------------------------------------------------------------

#[test]
fn deleting_tombstones_the_reminder_and_keeps_its_archive() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, daily_plan());
    resolve(&mut s, &c, 1, 10, done(Some(day_slot(0)))).expect("resolved");

    assert_eq!(
        s.delete(&c, rid(1), 9),
        Err(CommandError::Conflict { current: 1 })
    );
    s.delete(&c, rid(1), 1).expect("deleted");

    assert_eq!(s.get(rid(1)), Err(CommandError::NotFound));
    assert!(s.list().expect("listed").is_empty());
    assert_eq!(s.history(rid(1)), Err(CommandError::NotFound));
    assert_eq!(s.delete(&c, rid(1), 2), Err(CommandError::NotFound));
    assert_eq!(
        s.edit(&c, rid(1), 2, rename("Gone")),
        Err(CommandError::NotFound)
    );

    assert_eq!(s.storage.progress(rid(1)).expect("read"), None);
    assert_eq!(s.storage.resolutions(rid(1)).expect("read").len(), 1);
    let stored = s.storage.reminder(rid(1)).expect("read").expect("kept");
    assert_eq!(stored.meta.deleted_at, Some(c.now));
}

// --- resolve --------------------------------------------------------------

#[test]
fn resolving_a_one_off_completes_it() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, once_plan());

    let resolved = resolve(&mut s, &c, 1, 10, done(None)).expect("completed");
    let progress = resolved.progress.expect("a one-off has progress");
    assert_eq!(progress.state, ProgressState::Once { done_at: Some(c.now) });
    assert_eq!(progress.last_resolution, Some(xid(10)));
    assert_eq!(s.history(rid(1)).expect("history").len(), 1);

    assert_eq!(
        resolve(&mut s, &c, 1, 11, done(None)).unwrap_err(),
        CommandError::Transition(TransitionError::AlreadyDone)
    );
    // The refused command left nothing behind.
    assert_eq!(s.history(rid(1)).expect("history").len(), 1);
}

#[test]
fn a_one_off_takes_only_a_completion_without_a_slot() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, once_plan());

    for outcome in [skipped(day_slot(0)), done(Some(day_slot(0)))] {
        assert_eq!(
            resolve(&mut s, &c, 1, 10, outcome).unwrap_err(),
            CommandError::Invalid(Invalid::OutcomeNotAllowed)
        );
    }
}

#[test]
fn resolving_cadence_slots_follows_the_watermark() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, daily_plan());

    let first = resolve(&mut s, &c, 1, 10, done(Some(day_slot(0)))).expect("resolved");
    assert_eq!(
        watermark(&first.progress.expect("progress")),
        Watermark {
            through: Some(day_slot(0)),
            handled: 1,
        }
    );

    // Already handled, not a slot, and too far ahead.
    assert_eq!(
        resolve(&mut s, &c, 1, 11, done(Some(day_slot(0)))).unwrap_err(),
        CommandError::Transition(TransitionError::AlreadyHandled)
    );
    let not_a_slot = Slot(civil::date(2026, 11, 17).at(10, 0, 0, 0));
    assert_eq!(
        resolve(&mut s, &c, 1, 12, done(Some(not_a_slot))).unwrap_err(),
        CommandError::Transition(TransitionError::NotASlot)
    );
    assert_eq!(
        resolve(&mut s, &c, 1, 13, done(Some(day_slot(4)))).unwrap_err(),
        CommandError::NotResolvable
    );

    // A skip works like a completion, and passes over the slot in between.
    let skip = resolve(&mut s, &c, 1, 14, skipped(day_slot(2))).expect("skipped");
    assert_eq!(
        watermark(&skip.progress.expect("progress")),
        Watermark {
            through: Some(day_slot(2)),
            handled: 3,
        }
    );

    // The next pending slot is in the future, and can be resolved early. The one
    // after it cannot.
    resolve(&mut s, &c, 1, 15, done(Some(day_slot(3)))).expect("early");
    assert_eq!(
        resolve(&mut s, &c, 1, 16, done(Some(day_slot(5)))).unwrap_err(),
        CommandError::NotResolvable
    );
}

#[test]
fn a_floating_slot_is_judged_against_the_device_zone() {
    // The 18th at 09:00 is already past at UTC but still ahead at UTC-12.
    let now = "2026-11-18T12:00:00Z";

    let mut ahead = service();
    create(&mut ahead, 1, daily_plan());
    resolve(&mut ahead, &ctx_in(now, 0), 1, 10, done(Some(day_slot(2)))).expect("past at UTC");

    let mut behind = service();
    create(&mut behind, 1, daily_plan());
    assert_eq!(
        resolve(&mut behind, &ctx_in(now, -12), 1, 10, done(Some(day_slot(2)))).unwrap_err(),
        CommandError::NotResolvable
    );
}

#[test]
fn a_completion_without_a_slot_only_logs_it_on_a_cadence_or_tracker() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, daily_plan());
    create(&mut s, 2, tracker_plan());
    let before = progress_of(&s, 1);

    let on_cadence = resolve(&mut s, &c, 1, 10, done(None)).expect("logged");
    assert_eq!(on_cadence.progress, before);
    assert_eq!(progress_of(&s, 1), before);

    let on_tracker = resolve(&mut s, &c, 2, 11, done(None)).expect("logged");
    assert_eq!(on_tracker.progress, None);

    assert_eq!(s.history(rid(1)).expect("history").len(), 1);
    assert_eq!(s.history(rid(2)).expect("history").len(), 1);
}

#[test]
fn a_tracker_takes_only_a_completion_without_a_slot() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, tracker_plan());

    for outcome in [skipped(day_slot(0)), done(Some(day_slot(0)))] {
        assert_eq!(
            resolve(&mut s, &c, 1, 10, outcome).unwrap_err(),
            CommandError::Invalid(Invalid::OutcomeNotAllowed)
        );
    }
}

#[test]
fn a_used_resolution_id_is_refused() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, tracker_plan());
    create(&mut s, 2, tracker_plan());
    resolve(&mut s, &c, 1, 10, done(None)).expect("logged");

    assert_eq!(
        resolve(&mut s, &c, 1, 10, done(None)).unwrap_err(),
        CommandError::AlreadyExists
    );
    assert_eq!(
        resolve(&mut s, &c, 2, 10, done(None)).unwrap_err(),
        CommandError::AlreadyExists
    );
}

// --- undo -----------------------------------------------------------------

#[test]
fn undoing_the_latest_resolution_rolls_progress_back_and_retracts_it() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, daily_plan());
    let before = progress_of(&s, 1).expect("progress");

    resolve(&mut s, &c, 1, 10, done(Some(day_slot(1)))).expect("resolved");
    s.undo(&c, rid(1), xid(10)).expect("undone");

    let after = progress_of(&s, 1).expect("progress");
    assert_eq!(watermark(&after), watermark(&before));
    assert_eq!(after.last_resolution, None);
    assert!(s.history(rid(1)).expect("history").is_empty());
    assert_eq!(
        s.undo(&c, rid(1), xid(10)),
        Err(CommandError::NotFound)
    );
}

#[test]
fn only_the_latest_resolution_that_moved_progress_can_be_undone() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, daily_plan());
    resolve(&mut s, &c, 1, 10, done(Some(day_slot(0)))).expect("resolved");
    resolve(&mut s, &c, 1, 11, done(Some(day_slot(1)))).expect("resolved");

    assert_eq!(s.undo(&c, rid(1), xid(10)), Err(CommandError::NotLatest));
    s.undo(&c, rid(1), xid(11)).expect("the latest");
    // Undo is one level deep, so the earlier one cannot be undone either.
    assert_eq!(s.undo(&c, rid(1), xid(10)), Err(CommandError::NotLatest));
}

#[test]
fn a_log_only_resolution_can_be_undone_at_any_time() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, daily_plan());
    resolve(&mut s, &c, 1, 10, done(None)).expect("logged");
    resolve(&mut s, &c, 1, 11, done(Some(day_slot(0)))).expect("resolved");
    let progress = progress_of(&s, 1);

    s.undo(&c, rid(1), xid(10)).expect("a log-only entry");

    assert_eq!(progress_of(&s, 1), progress);
    let ids: Vec<_> = s
        .history(rid(1))
        .expect("history")
        .iter()
        .map(|resolution| resolution.id)
        .collect();
    assert_eq!(ids, vec![xid(11)]);
}

#[test]
fn undoing_a_one_off_reopens_it() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, once_plan());
    resolve(&mut s, &c, 1, 10, done(None)).expect("completed");

    s.undo(&c, rid(1), xid(10)).expect("undone");

    let progress = progress_of(&s, 1).expect("progress");
    assert_eq!(progress.state, ProgressState::Once { done_at: None });
    assert!(s.history(rid(1)).expect("history").is_empty());
}

#[test]
fn a_tracker_log_can_be_undone() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, tracker_plan());
    resolve(&mut s, &c, 1, 10, done(None)).expect("logged");

    s.undo(&c, rid(1), xid(10)).expect("undone");
    assert!(s.history(rid(1)).expect("history").is_empty());
}

#[test]
fn undo_names_the_right_reminder() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, tracker_plan());
    create(&mut s, 2, tracker_plan());
    resolve(&mut s, &c, 1, 10, done(None)).expect("logged");

    assert_eq!(s.undo(&c, rid(2), xid(10)), Err(CommandError::NotFound));
    assert_eq!(s.undo(&c, rid(1), xid(99)), Err(CommandError::NotFound));
}

// --- queries --------------------------------------------------------------

#[test]
fn history_is_newest_first_and_leaves_out_retracted_entries() {
    let mut s = service();
    create(&mut s, 1, tracker_plan());
    for (n, time) in [
        (10, "2026-11-18T08:00:00Z"),
        (11, "2026-11-18T09:00:00Z"),
        (12, "2026-11-18T10:00:00Z"),
    ] {
        resolve(&mut s, &ctx(time), 1, n, done(None)).expect("logged");
    }
    s.undo(&ctx("2026-11-18T11:00:00Z"), rid(1), xid(11))
        .expect("undone");

    let ids: Vec<_> = s
        .history(rid(1))
        .expect("history")
        .iter()
        .map(|resolution| resolution.id)
        .collect();
    assert_eq!(ids, vec![xid(12), xid(10)]);
}

#[test]
fn listing_is_oldest_first_and_leaves_out_deleted_reminders() {
    let mut s = service();
    for (id, time) in [
        (3, "2026-11-18T08:00:00Z"),
        (1, "2026-11-18T09:00:00Z"),
        (2, "2026-11-18T10:00:00Z"),
    ] {
        s.create(&ctx(time), rid(id), draft(tracker_plan()))
            .expect("created");
    }
    let ids = |s: &Service| -> Vec<ReminderId> {
        s.list()
            .expect("listed")
            .iter()
            .map(|state| state.reminder.id)
            .collect()
    };
    assert_eq!(ids(&s), vec![rid(3), rid(1), rid(2)]);

    s.delete(&ctx(NOW), rid(1), 1).expect("deleted");
    assert_eq!(ids(&s), vec![rid(3), rid(2)]);
}

#[test]
fn upcoming_is_ordered_by_when_slots_fire_not_by_slot_time() {
    let mut s = service();
    // Floating slots are read at UTC+10 here.
    let c = ctx_in(NOW, 10);
    // 2026-11-20 09:00 at UTC+10 fires 2026-11-19T23:00Z.
    s.create(&c, rid(1), draft(once_plan())).expect("created");
    // Overdue: 2026-11-16 09:00 at UTC+10 fired 2026-11-15T23:00Z.
    s.create(&c, rid(2), draft(daily_plan())).expect("created");
    // The slot is earlier than reminder 1's, but it fires later: 20:00 in New
    // York is 2026-11-20T01:00Z.
    let new_york = once_due(
        civil::date(2026, 11, 19).at(20, 0, 0, 0),
        Zone::Fixed("America/New_York".to_owned()),
    );
    s.create(&c, rid(3), draft(new_york)).expect("created");

    let list = s.upcoming(&c, 10).expect("upcoming");
    let order: Vec<_> = list.upcoming.iter().map(|item| item.reminder_id).collect();
    assert_eq!(order, vec![rid(2), rid(1), rid(3)]);
    let instants: Vec<_> = list.upcoming.iter().map(|item| item.at).collect();
    assert_eq!(
        instants,
        vec![
            ts("2026-11-15T23:00:00Z"),
            ts("2026-11-19T23:00:00Z"),
            ts("2026-11-20T01:00:00Z"),
        ]
    );
    assert!(list.unresolved.is_empty());

    assert_eq!(s.upcoming(&c, 2).expect("upcoming").upcoming.len(), 2);
}

#[test]
fn upcoming_moves_to_the_next_slot_once_one_is_resolved() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, daily_plan());
    assert_eq!(
        s.upcoming(&c, 10).expect("upcoming").upcoming[0].slot,
        day_slot(0)
    );

    resolve(&mut s, &c, 1, 10, done(Some(day_slot(0)))).expect("resolved");
    assert_eq!(
        s.upcoming(&c, 10).expect("upcoming").upcoming[0].slot,
        day_slot(1)
    );
}

#[test]
fn upcoming_leaves_out_what_cannot_alert() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, once_plan());
    resolve(&mut s, &c, 1, 10, done(None)).expect("completed");
    create(&mut s, 2, once_plan());
    s.edit(&c, rid(2), 1, archive(true)).expect("archived");
    create(&mut s, 3, once_plan());
    s.delete(&c, rid(3), 1).expect("deleted");
    create(&mut s, 4, tracker_plan());
    create(
        &mut s,
        5,
        Plan::Once { due: None },
    );
    create(&mut s, 6, once_plan());

    let order: Vec<_> = s
        .upcoming(&c, 10)
        .expect("upcoming")
        .upcoming
        .iter()
        .map(|item| item.reminder_id)
        .collect();
    assert_eq!(order, vec![rid(6)]);
}

#[test]
fn upcoming_reports_reminders_it_cannot_schedule() {
    let mut s = service();
    let c = ctx(NOW);
    let meta = Meta {
        revision: 1,
        created_at: c.now,
        updated_at: c.now,
        deleted_at: None,
    };
    let reminder = |id, plan| Reminder {
        id: rid(id),
        title: title("Broken"),
        notes: String::new(),
        plan,
        archived_at: None,
        meta: meta.clone(),
    };

    // A zone the database does not know, which `create` would have refused.
    let bad_zone = reminder(
        1,
        once_due(
            civil::date(2026, 11, 20).at(9, 0, 0, 0),
            Zone::Fixed("Not/AZone".to_owned()),
        ),
    );
    let progress = Progress::initial(rid(1), &bad_zone.plan, c.now).map(|progress| {
        ProgressWrite::Put(progress, Expect::Absent)
    });
    s.storage
        .commit(WriteSet {
            reminder: Some((bad_zone, Expect::Absent)),
            progress,
            resolution: None,
        })
        .expect("stored");

    // A schedule with no progress row.
    s.storage
        .commit(WriteSet {
            reminder: Some((reminder(2, daily_plan()), Expect::Absent)),
            progress: None,
            resolution: None,
        })
        .expect("stored");

    let list = s.upcoming(&c, 10).expect("upcoming");
    assert!(list.upcoming.is_empty());
    assert_eq!(
        list.unresolved,
        vec![
            Unresolved {
                reminder_id: rid(1),
                reason: UnresolvedReason::Zone(ZoneError::UnknownZone("Not/AZone".to_owned())),
            },
            Unresolved {
                reminder_id: rid(2),
                reason: UnresolvedReason::MissingProgress,
            },
        ]
    );
}

// --- alarms ---------------------------------------------------------------

fn slots_of(list: &super::UpcomingList) -> Vec<Slot> {
    list.upcoming.iter().map(|item| item.slot).collect()
}

#[test]
fn alarms_skip_a_slot_that_already_passed_unresolved() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, daily_plan());

    // The 16th, 17th and 18th have all passed unresolved. The oldest is still
    // what is pending, but the alarm is for the next one to fire.
    assert_eq!(s.upcoming(&c, 10).expect("upcoming").upcoming[0].slot, day_slot(0));
    let alarms = s.alarms(&c, 10).expect("alarms");
    assert_eq!(slots_of(&alarms), vec![day_slot(3)]);
    assert_eq!(alarms.upcoming[0].at, ts("2026-11-19T09:00:00Z"));
    assert!(alarms.unresolved.is_empty());
}

#[test]
fn a_slot_firing_right_now_is_not_the_next_alarm() {
    let mut s = service();
    create(&mut s, 1, daily_plan());

    let before = s.alarms(&ctx("2026-11-19T08:59:59Z"), 10).expect("alarms");
    assert_eq!(slots_of(&before), vec![day_slot(3)]);
    let at = s.alarms(&ctx("2026-11-19T09:00:00Z"), 10).expect("alarms");
    assert_eq!(slots_of(&at), vec![day_slot(4)]);
}

#[test]
fn alarms_for_a_one_off_cover_only_a_due_time_still_ahead() {
    let mut s = service();
    create(&mut s, 1, once_plan());

    let ahead = s.alarms(&ctx(NOW), 10).expect("alarms");
    assert_eq!(ahead.upcoming.len(), 1);
    assert_eq!(ahead.upcoming[0].at, ts("2026-11-20T09:00:00Z"));

    let passed = s.alarms(&ctx("2026-11-21T00:00:00Z"), 10).expect("alarms");
    assert!(passed.upcoming.is_empty());
    // Still pending, only not an alarm any more.
    assert_eq!(
        s.upcoming(&ctx("2026-11-21T00:00:00Z"), 10)
            .expect("upcoming")
            .upcoming
            .len(),
        1
    );
}

#[test]
fn alarms_for_a_counted_recurrence_end_with_its_last_slot() {
    let mut s = service();
    let counted = daily(Zone::Floating, 9, Some(End::Count(NonZeroU32::new(3).expect("three"))));
    create(&mut s, 1, counted);

    // After the second slot, the third is still to come.
    let before_last = s.alarms(&ctx("2026-11-17T12:00:00Z"), 10).expect("alarms");
    assert_eq!(slots_of(&before_last), vec![day_slot(2)]);
    // After the third has passed unresolved, there is nothing left.
    let after_last = s.alarms(&ctx(NOW), 10).expect("alarms");
    assert!(after_last.upcoming.is_empty());
}

#[test]
fn alarms_read_a_fixed_zone_in_that_zone() {
    let mut s = service();
    create(
        &mut s,
        1,
        daily(Zone::Fixed("America/New_York".to_owned()), 9, None),
    );

    // 12:00Z is 07:00 in New York, so the 09:00 slot of the 18th is still ahead.
    let alarms = s.alarms(&ctx(NOW), 10).expect("alarms");
    assert_eq!(slots_of(&alarms), vec![day_slot(2)]);
    assert_eq!(alarms.upcoming[0].at, ts("2026-11-18T14:00:00Z"));
}

#[test]
fn alarms_leave_out_what_cannot_alert_and_are_ordered_by_when_they_fire() {
    let mut s = service();
    let c = ctx(NOW);
    create(&mut s, 1, daily_plan());
    create(&mut s, 2, once_plan());
    create(
        &mut s,
        3,
        once_due(civil::date(2026, 11, 19).at(8, 0, 0, 0), Zone::Floating),
    );
    create(&mut s, 4, tracker_plan());
    create(&mut s, 5, once_plan());
    s.edit(&c, rid(5), 1, archive(true)).expect("archived");
    create(&mut s, 6, once_plan());
    s.delete(&c, rid(6), 1).expect("deleted");
    create(&mut s, 7, once_plan());
    resolve(&mut s, &c, 7, 10, done(None)).expect("completed");

    let order: Vec<_> = s
        .alarms(&c, 10)
        .expect("alarms")
        .upcoming
        .iter()
        .map(|item| item.reminder_id)
        .collect();
    assert_eq!(order, vec![rid(3), rid(1), rid(2)]);

    assert_eq!(s.alarms(&c, 2).expect("alarms").upcoming.len(), 2);
}

#[test]
fn alarms_report_reminders_they_cannot_schedule() {
    let mut s = service();
    let c = ctx(NOW);
    let bad_zone = Reminder {
        id: rid(1),
        title: title("Broken"),
        notes: String::new(),
        plan: once_due(
            civil::date(2026, 11, 20).at(9, 0, 0, 0),
            Zone::Fixed("Not/AZone".to_owned()),
        ),
        archived_at: None,
        meta: Meta {
            revision: 1,
            created_at: c.now,
            updated_at: c.now,
            deleted_at: None,
        },
    };
    let progress = Progress::initial(rid(1), &bad_zone.plan, c.now)
        .map(|progress| ProgressWrite::Put(progress, Expect::Absent));
    s.storage
        .commit(WriteSet {
            reminder: Some((bad_zone, Expect::Absent)),
            progress,
            resolution: None,
        })
        .expect("stored");

    let list = s.alarms(&c, 10).expect("alarms");
    assert!(list.upcoming.is_empty());
    assert_eq!(
        list.unresolved,
        vec![Unresolved {
            reminder_id: rid(1),
            reason: UnresolvedReason::Zone(ZoneError::UnknownZone("Not/AZone".to_owned())),
        }]
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn alarms_find_the_first_slot_that_fires_after_now(
        rec in any_recurrence(),
        offset_seconds in 0..(700 * 86_400i64),
    ) {
        let mut s = service();
        let plan = Plan::Ongoing {
            cadence: Some(Cadence { recurrence: rec.clone(), alert: alert() }),
        };
        // A recurrence that ends before it starts is refused. Nothing to check there.
        prop_assume!(s.create(&ctx(NOW), rid(1), draft(plan)).is_ok());

        // Floating slots in a UTC device zone fire at their own clock time.
        let start = TimeZone::UTC
            .to_timestamp(rec.starts_on.to_datetime(civil::Time::midnight()))
            .expect("a representable instant");
        let now = start
            .checked_add(SignedDuration::from_secs(offset_seconds))
            .expect("a representable instant");
        let c = Context { now, device_zone: TimeZone::UTC };

        let last_day = rec.starts_on + 800.days();
        let expected = oracle(&rec, last_day).into_iter().find(|slot| {
            TimeZone::UTC.to_timestamp(slot.0).expect("a representable instant") > now
        });
        let actual = slots_of(&s.alarms(&c, 1).expect("alarms")).first().copied();

        match expected {
            Some(_) => prop_assert_eq!(actual, expected),
            // The enumeration stopped at its horizon, so a slot just past it is fine.
            None => prop_assert!(actual.is_none_or(|Slot(slot)| slot.date() > last_day)),
        }
    }
}

// --- command sequences ----------------------------------------------------

/// A plan of each kind: 0 a one-off, 1 a daily cadence, 2 a tracker.
fn plan_of_kind(kind: u8) -> Plan {
    match kind {
        0 => once_plan(),
        1 => daily_plan(),
        _ => tracker_plan(),
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Op {
    Resolve {
        target: usize,
        slot_day: Option<u8>,
        skip: bool,
    },
    Undo {
        target: usize,
    },
    SetPlan {
        target: usize,
        kind: u8,
    },
    Archive {
        target: usize,
        on: bool,
    },
    Delete {
        target: usize,
    },
}

pub(crate) fn any_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0..3usize, prop::option::of(0..6u8), any::<bool>()).prop_map(
            |(target, slot_day, skip)| Op::Resolve {
                target,
                slot_day,
                skip,
            }
        ),
        (0..3usize).prop_map(|target| Op::Undo { target }),
        (0..3usize, 0..3u8).prop_map(|(target, kind)| Op::SetPlan { target, kind }),
        (0..3usize, any::<bool>()).prop_map(|(target, on)| Op::Archive { target, on }),
        (0..3usize).prop_map(|target| Op::Delete { target }),
    ]
}

pub(crate) fn ids() -> [ReminderId; 3] {
    [rid(1), rid(2), rid(3)]
}

/// Creates three reminders: a one-off, a daily cadence and a tracker.
pub(crate) fn seed<S: Storage>(s: &mut LocalReminders<S>, c: &Context) {
    for (index, id) in ids().iter().enumerate() {
        let kind = u8::try_from(index).expect("small");
        s.create(c, *id, draft(plan_of_kind(kind))).expect("created");
    }
}

/// What [`apply`] remembers between operations.
pub(crate) struct Driver {
    last_resolution: [Option<ResolutionId>; 3],
    next_resolution: u128,
}

impl Driver {
    pub(crate) fn new() -> Self {
        Self {
            last_resolution: [None; 3],
            next_resolution: 100,
        }
    }
}

/// Runs one operation against the three seeded reminders.
pub(crate) fn apply<S: Storage>(
    s: &mut LocalReminders<S>,
    c: &Context,
    driver: &mut Driver,
    op: Op,
) -> Result<(), CommandError> {
    let ids = ids();
    match op {
        Op::Resolve {
            target,
            slot_day,
            skip,
        } => {
            driver.next_resolution += 1;
            let outcome = match (slot_day, skip) {
                (Some(day), false) => done(Some(day_slot(day))),
                (Some(day), true) => skipped(day_slot(day)),
                (None, _) => done(None),
            };
            let resolution = xid(driver.next_resolution);
            s.resolve(c, ids[target], resolution, outcome, None)?;
            driver.last_resolution[target] = Some(resolution);
            Ok(())
        }
        Op::Undo { target } => match driver.last_resolution[target] {
            Some(resolution) => s.undo(c, ids[target], resolution),
            None => Ok(()),
        },
        Op::SetPlan { target, kind } => {
            let state = s.get(ids[target])?;
            s.edit(
                c,
                ids[target],
                state.reminder.meta.revision,
                replace_plan(plan_of_kind(kind)),
            )
            .map(|_| ())
        }
        Op::Archive { target, on } => {
            let state = s.get(ids[target])?;
            s.edit(c, ids[target], state.reminder.meta.revision, archive(on))
                .map(|_| ())
        }
        Op::Delete { target } => {
            let state = s.get(ids[target])?;
            s.delete(c, ids[target], state.reminder.meta.revision)
        }
    }
}

/// Reminder and progress revisions per reminder.
type Revisions = HashMap<ReminderId, (u64, Option<u64>)>;

fn check_consistent(
    s: &Service,
    c: &Context,
    previous: &mut Revisions,
) -> Result<(), TestCaseError> {
    for state in s.list().expect("listed") {
        let id = state.reminder.id;
        let matches_plan = match (&state.reminder.plan, &state.progress) {
            (
                Plan::Once { .. },
                Some(Progress {
                    state: ProgressState::Once { .. },
                    ..
                }),
            ) => true,
            (
                Plan::Ongoing { cadence: Some(_) },
                Some(Progress {
                    state: ProgressState::Cadence { .. },
                    ..
                }),
            ) => true,
            (Plan::Ongoing { cadence: None }, None) => true,
            _ => false,
        };
        prop_assert!(matches_plan, "progress does not match the plan: {:?}", state);

        let reminder_revision = state.reminder.meta.revision;
        let progress_revision = state.progress.as_ref().map(|progress| progress.revision);
        if let Some((old_reminder, old_progress)) = previous.get(&id) {
            prop_assert!(reminder_revision >= *old_reminder);
            if let (Some(old), Some(new)) = (old_progress, progress_revision) {
                prop_assert!(new >= *old);
            }
        }
        previous.insert(id, (reminder_revision, progress_revision));
    }

    let list = s.upcoming(c, 100).expect("upcoming");
    prop_assert!(list.upcoming.windows(2).all(|pair| pair[0].at <= pair[1].at));
    prop_assert!(list.unresolved.is_empty());
    for item in &list.upcoming {
        let state = s.get(item.reminder_id).expect("an upcoming reminder exists");
        prop_assert!(state.reminder.archived_at.is_none());
        let completed = matches!(
            state.progress,
            Some(Progress {
                state: ProgressState::Once { done_at: Some(_) },
                ..
            })
        );
        prop_assert!(!completed);
    }
    Ok(())
}

proptest! {
    #[test]
    fn any_sequence_of_commands_keeps_the_state_consistent(
        ops in prop::collection::vec(any_op(), 1..40),
    ) {
        let mut s = service();
        let c = ctx(NOW);
        seed(&mut s, &c);
        let mut driver = Driver::new();
        let mut previous = Revisions::new();
        check_consistent(&s, &c, &mut previous)?;

        for op in ops {
            let _ = apply(&mut s, &c, &mut driver, op);
            check_consistent(&s, &c, &mut previous)?;
        }
    }
}
