//! Tests for the [`Progress`] transitions.

use std::num::NonZeroU32;

use jiff::{Timestamp, ToSpan, civil};
use proptest::prelude::*;
use uuid::Uuid;

use super::TransitionError;
use crate::model::{
    Alert, AlertStyle, Cadence, ClockTimes, Due, End, Plan, Progress, ProgressState, Recurrence,
    ReminderId, ResolutionId, Rule, Slot, TimesOfDay, Watermark, When, Zone,
};

fn reminder_id() -> ReminderId {
    ReminderId(Uuid::from_u128(1))
}

fn resolution_id(n: u128) -> ResolutionId {
    ResolutionId(Uuid::from_u128(n))
}

fn now(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("a valid timestamp")
}

fn alert() -> Alert {
    Alert {
        style: AlertStyle::Notification { sticky: false },
        nudge: None,
    }
}

fn slot(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Slot {
    Slot(civil::date(year, month, day).at(hour, minute, 0, 0))
}

fn once_plan() -> Plan {
    Plan::Once {
        due: Some(Due {
            when: When {
                local: civil::date(2026, 11, 20).at(9, 0, 0, 0),
                zone: Zone::Floating,
            },
            alert: alert(),
        }),
    }
}

/// Daily at 09:00 from 2026-11-16, repeating every `every` days.
fn daily_plan(every: u32, end: Option<End>) -> Plan {
    Plan::Ongoing {
        cadence: Some(Cadence {
            recurrence: Recurrence {
                starts_on: civil::date(2026, 11, 16),
                zone: Zone::Floating,
                rule: Rule::Daily {
                    every: NonZeroU32::new(every).expect("every is not zero"),
                },
                at: TimesOfDay::At(
                    ClockTimes::new([civil::time(9, 0, 0, 0)]).expect("one time"),
                ),
                end,
            },
            alert: alert(),
        }),
    }
}

fn tracker_plan() -> Plan {
    Plan::Ongoing { cadence: None }
}

fn initial(plan: &Plan) -> Progress {
    Progress::initial(reminder_id(), plan, now(1)).expect("the plan has progress")
}

/// Resolves with resolution id 100 at `seconds`.
fn resolve(
    progress: &Progress,
    plan: &Plan,
    slot: Option<Slot>,
    seconds: i64,
) -> Result<Progress, TransitionError> {
    progress.resolve(plan, slot, resolution_id(100), now(seconds))
}

fn watermark(progress: &Progress) -> Watermark {
    match &progress.state {
        ProgressState::Cadence { current, .. } => *current,
        ProgressState::Once { .. } => panic!("expected cadence progress"),
    }
}

#[test]
fn initial_progress_matches_the_plan() {
    let once = initial(&once_plan());
    assert_eq!(once.state, ProgressState::Once { done_at: None });
    assert_eq!(once.last_resolution, None);
    assert_eq!(once.revision, 1);
    assert_eq!(once.updated_at, now(1));

    let cadence = initial(&daily_plan(1, None));
    assert_eq!(
        cadence.state,
        ProgressState::Cadence {
            current: Watermark {
                through: None,
                handled: 0,
            },
            previous: None,
        }
    );

    assert_eq!(Progress::initial(reminder_id(), &tracker_plan(), now(1)), None);
}

#[test]
fn completing_a_one_off_records_when_and_bumps_the_revision() {
    let plan = once_plan();
    let done = resolve(&initial(&plan), &plan, None, 5).expect("open");
    assert_eq!(done.state, ProgressState::Once { done_at: Some(now(5)) });
    assert_eq!(done.revision, 2);
    assert_eq!(done.updated_at, now(5));
}

#[test]
fn a_one_off_cannot_be_completed_twice() {
    let plan = once_plan();
    let done = resolve(&initial(&plan), &plan, None, 5).expect("open");
    assert_eq!(resolve(&done, &plan, None, 6), Err(TransitionError::AlreadyDone));
}

#[test]
fn undoing_a_one_off_reopens_it() {
    let plan = once_plan();
    let open = initial(&plan);
    assert_eq!(open.undo(now(2)), Err(TransitionError::NothingToUndo));

    let reopened = resolve(&open, &plan, None, 5)
        .and_then(|done| done.undo(now(6)))
        .expect("done then undone");
    assert_eq!(reopened.state, ProgressState::Once { done_at: None });
    assert_eq!(reopened.revision, 3);
}

#[test]
fn a_one_offs_pending_slot_is_its_due_time_until_it_is_done() {
    let plan = once_plan();
    let open = initial(&plan);
    assert_eq!(open.next_pending(&plan), Some(slot(2026, 11, 20, 9, 0)));

    let done = resolve(&open, &plan, None, 5).expect("open");
    assert_eq!(done.next_pending(&plan), None);
}

#[test]
fn resolving_the_first_slot_moves_the_watermark_and_remembers_the_old_one() {
    let plan = daily_plan(1, None);
    let first = initial(&plan);
    let resolved =
        resolve(&first, &plan, Some(slot(2026, 11, 16, 9, 0)), 5).expect("the first slot");

    assert_eq!(
        resolved.state,
        ProgressState::Cadence {
            current: Watermark {
                through: Some(slot(2026, 11, 16, 9, 0)),
                handled: 1,
            },
            previous: Some(watermark(&first)),
        }
    );
    assert_eq!(resolved.revision, 2);
}

#[test]
fn resolving_a_later_slot_counts_the_slots_it_passes_over() {
    let plan = daily_plan(1, None);
    let resolved = resolve(&initial(&plan), &plan, Some(slot(2026, 11, 18, 9, 0)), 5)
        .expect("the third slot");

    assert_eq!(
        watermark(&resolved),
        Watermark {
            through: Some(slot(2026, 11, 18, 9, 0)),
            handled: 3,
        }
    );
}

#[test]
fn handled_slots_cannot_be_resolved_again() {
    let plan = daily_plan(1, None);
    let resolved = resolve(&initial(&plan), &plan, Some(slot(2026, 11, 17, 9, 0)), 5)
        .expect("the second slot");

    for handled in [slot(2026, 11, 17, 9, 0), slot(2026, 11, 16, 9, 0)] {
        assert_eq!(
            resolve(&resolved, &plan, Some(handled), 6),
            Err(TransitionError::AlreadyHandled)
        );
    }
}

#[test]
fn only_real_slots_can_be_resolved() {
    let plan = daily_plan(1, None);
    let open = initial(&plan);

    assert_eq!(
        resolve(&open, &plan, Some(slot(2026, 11, 17, 10, 0)), 5),
        Err(TransitionError::NotASlot)
    );
    assert_eq!(resolve(&open, &plan, None, 5), Err(TransitionError::NotASlot));
}

#[test]
fn undo_restores_the_previous_watermark_one_level_deep() {
    let plan = daily_plan(1, None);
    let first = initial(&plan);
    let resolved = resolve(&first, &plan, Some(slot(2026, 11, 17, 9, 0)), 5)
        .expect("the second slot");

    let undone = resolved.undo(now(6)).expect("one resolution to undo");
    assert_eq!(watermark(&undone), watermark(&first));
    assert_eq!(undone.undo(now(7)), Err(TransitionError::NothingToUndo));
}

#[test]
fn resolving_records_the_resolution_and_undoing_forgets_it() {
    let plan = daily_plan(1, None);
    let resolved = initial(&plan)
        .resolve(&plan, Some(slot(2026, 11, 16, 9, 0)), resolution_id(7), now(5))
        .expect("the first slot");
    assert_eq!(resolved.last_resolution, Some(resolution_id(7)));

    let undone = resolved.undo(now(6)).expect("one resolution to undo");
    assert_eq!(undone.last_resolution, None);
}

#[test]
fn progress_of_the_wrong_kind_is_refused() {
    let once = initial(&once_plan());
    let cadence = initial(&daily_plan(1, None));

    assert_eq!(
        resolve(&once, &daily_plan(1, None), Some(slot(2026, 11, 16, 9, 0)), 5),
        Err(TransitionError::WrongPlan)
    );
    assert_eq!(
        resolve(&cadence, &once_plan(), None, 5),
        Err(TransitionError::WrongPlan)
    );
    assert_eq!(
        resolve(&cadence, &tracker_plan(), None, 5),
        Err(TransitionError::WrongPlan)
    );
}

#[test]
fn the_pending_slot_follows_the_watermark() {
    let plan = daily_plan(1, None);
    let first = initial(&plan);
    assert_eq!(first.next_pending(&plan), Some(slot(2026, 11, 16, 9, 0)));

    let resolved =
        resolve(&first, &plan, Some(slot(2026, 11, 16, 9, 0)), 5).expect("the first slot");
    assert_eq!(resolved.next_pending(&plan), Some(slot(2026, 11, 17, 9, 0)));
}

#[test]
fn nothing_is_pending_once_a_counted_recurrence_is_exhausted() {
    let plan = daily_plan(1, Some(End::Count(NonZeroU32::new(2).expect("not zero"))));
    let exhausted = resolve(&initial(&plan), &plan, Some(slot(2026, 11, 17, 9, 0)), 5)
        .expect("the second and last slot");

    assert_eq!(exhausted.next_pending(&plan), None);
    assert_eq!(
        resolve(&exhausted, &plan, Some(slot(2026, 11, 18, 9, 0)), 6),
        Err(TransitionError::NotASlot)
    );
}

proptest! {
    #[test]
    fn resolving_forward_tracks_the_slots_passed_over_and_undo_reverses_the_last(
        every in 1..=3u32,
        strides in prop::collection::vec(1usize..=4, 1..8),
    ) {
        let plan = daily_plan(every, None);
        let mut progress = initial(&plan);
        let mut handled = 0usize;

        for (step, stride) in strides.iter().enumerate() {
            handled += stride;
            // The nth slot of a daily recurrence is a known number of days on.
            let days = i32::try_from((handled - 1) * every as usize).expect("a small count");
            let target = Slot(
                (civil::date(2026, 11, 16) + days.days()).at(9, 0, 0, 0),
            );
            let before = progress.clone();

            progress = resolve(
                &before,
                &plan,
                Some(target),
                i64::try_from(step).expect("small") + 2,
            )
            .expect("the target is a later slot");

            prop_assert_eq!(
                watermark(&progress),
                Watermark {
                    through: Some(target),
                    handled: u32::try_from(handled).expect("a small count"),
                }
            );
            prop_assert_eq!(progress.revision, before.revision + 1);
            let ProgressState::Cadence { previous, .. } = &progress.state else {
                panic!("expected cadence progress");
            };
            prop_assert_eq!(previous, &Some(watermark(&before)));

            let undone = progress.undo(now(100)).expect("one resolution to undo");
            prop_assert_eq!(watermark(&undone), watermark(&before));
        }
    }
}
