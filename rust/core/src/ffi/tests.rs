//! Tests for the FFI layer: conversions to and from the model, the error
//! mapping, and the exported store driven through its Rust API.

use std::time::Duration;

use jiff::Timestamp;
use proptest::prelude::*;

use super::{
    Alert, AlertStyle, Cadence, CallContext, Due, End, MonthlyOn, Nth, Nudge, Outcome, Plan,
    ProgressState, Recurrence, ReminderDraft, ReminderEdit, ReminderError, ReminderStore, Rule,
    TimesOfDay, UnresolvedReason, Watermark, Weekday, When, Zone, new_reminder_id,
    new_resolution_id,
};
use crate::commands::{CommandError, Invalid, StorageError};
use crate::instant::ZoneError;
use crate::model as domain;
use crate::progress::TransitionError;
use crate::store::tests::{any_plan, any_slot};

// --- conversions ------------------------------------------------------------

proptest! {
    #[test]
    fn a_plan_survives_the_trip_through_the_ffi_types(plan in any_plan()) {
        let across = Plan::from(plan.clone());
        prop_assert_eq!(domain::Plan::try_from(across), Ok(plan));
    }

    #[test]
    fn an_outcome_survives_the_trip_through_the_ffi_types(
        slot in any_slot(),
        has_slot in any::<bool>(),
        skipped in any::<bool>(),
    ) {
        let outcome = match (skipped, has_slot) {
            (true, _) => domain::Outcome::Skipped { slot },
            (false, true) => domain::Outcome::Done { slot: Some(slot) },
            (false, false) => domain::Outcome::Done { slot: None },
        };
        let across = Outcome::from(outcome.clone());
        prop_assert_eq!(domain::Outcome::try_from(across), Ok(outcome));
    }
}

fn invalid_input<T: std::fmt::Debug>(result: Result<T, ReminderError>) {
    assert!(
        matches!(result, Err(ReminderError::InvalidInput { .. })),
        "expected InvalidInput, got {result:?}"
    );
}

fn alert() -> Alert {
    Alert {
        style: AlertStyle::Notification { sticky: false },
        nudge: None,
    }
}

fn daily_with(rule: Rule, at: TimesOfDay, end: Option<End>) -> Plan {
    Plan::Ongoing {
        cadence: Some(Cadence {
            recurrence: Recurrence {
                starts_on: "2026-11-16".to_owned(),
                zone: Zone::Floating,
                rule,
                at,
                end,
            },
            alert: alert(),
        }),
    }
}

fn at_nine() -> TimesOfDay {
    TimesOfDay::At {
        times: vec!["09:00:00".to_owned()],
    }
}

#[test]
fn input_the_model_would_refuse_is_refused_at_the_boundary() {
    let convert = |plan| domain::Plan::try_from(plan);

    // A rule that selects nothing, or counts nothing.
    invalid_input(convert(daily_with(Rule::Weekly { every: 1, on: vec![] }, at_nine(), None)));
    invalid_input(convert(daily_with(Rule::Daily { every: 0 }, at_nine(), None)));
    invalid_input(convert(daily_with(
        Rule::Monthly {
            every: 1,
            on: MonthlyOn::Day { day: 40 },
        },
        at_nine(),
        None,
    )));
    invalid_input(convert(daily_with(
        Rule::Daily { every: 1 },
        at_nine(),
        Some(End::Count { count: 0 }),
    )));

    // Times of day that are empty, reversed or not times.
    invalid_input(convert(daily_with(
        Rule::Daily { every: 1 },
        TimesOfDay::At { times: vec![] },
        None,
    )));
    invalid_input(convert(daily_with(
        Rule::Daily { every: 1 },
        TimesOfDay::At {
            times: vec!["not a time".to_owned()],
        },
        None,
    )));
    invalid_input(convert(daily_with(
        Rule::Daily { every: 1 },
        TimesOfDay::Every {
            every: Duration::from_secs(3600),
            start: "10:00:00".to_owned(),
            end: "09:00:00".to_owned(),
        },
        None,
    )));
    invalid_input(convert(daily_with(
        Rule::Daily { every: 1 },
        TimesOfDay::Every {
            every: Duration::ZERO,
            start: "09:00:00".to_owned(),
            end: "10:00:00".to_owned(),
        },
        None,
    )));

    // Dates that are not dates, and a nudge that never advances.
    let mut bad_date = daily_with(Rule::Daily { every: 1 }, at_nine(), None);
    if let Plan::Ongoing {
        cadence: Some(cadence),
    } = &mut bad_date
    {
        cadence.recurrence.starts_on = "2026-13-45".to_owned();
    }
    invalid_input(convert(bad_date));
    invalid_input(domain::Alert::try_from(Alert {
        style: AlertStyle::Alarm,
        nudge: Some(Nudge {
            every: Duration::ZERO,
            limit: None,
        }),
    }));
    invalid_input(domain::Outcome::try_from(Outcome::Skipped {
        slot: "yesterday".to_owned(),
    }));
}

#[test]
fn commands_with_a_blank_title_or_a_bad_id_are_refused() {
    let store = ReminderStore::open_in_memory().expect("opened");
    let draft = |title: &str| ReminderDraft {
        title: title.to_owned(),
        notes: String::new(),
        plan: Plan::Ongoing { cadence: None },
    };

    invalid_input(store.create(context(NOW_MS), id(1), draft("   ")));
    invalid_input(store.create(context(NOW_MS), "not-a-uuid".to_owned(), draft("Fine")));
    invalid_input(store.get("not-a-uuid".to_owned()));

    let mut at_the_phone = context(NOW_MS);
    at_the_phone.device_zone = "Not/AZone".to_owned();
    invalid_input(store.create(at_the_phone, id(1), draft("Fine")));
    invalid_input(store.edit(
        context(NOW_MS),
        id(1),
        1,
        ReminderEdit {
            title: Some(" ".to_owned()),
            notes: None,
            plan: None,
            archived: None,
        },
    ));
}

// --- the error mapping ------------------------------------------------------

#[test]
fn command_errors_keep_their_meaning_across_the_boundary() {
    use ReminderError as E;
    let cases = [
        (CommandError::NotFound, E::NotFound),
        (CommandError::AlreadyExists, E::AlreadyExists),
        (CommandError::Conflict { current: 3 }, E::Conflict { current: 3 }),
        (CommandError::Archived, E::Archived),
        (
            CommandError::Invalid(Invalid::UnknownZone("X/Y".to_owned())),
            E::UnknownZone {
                name: "X/Y".to_owned(),
            },
        ),
        (CommandError::Invalid(Invalid::EndsBeforeStart), E::EndsBeforeStart),
        (CommandError::Invalid(Invalid::OutcomeNotAllowed), E::OutcomeNotAllowed),
        (CommandError::NotResolvable, E::NotResolvable),
        (CommandError::NotLatest, E::NotLatest),
        (CommandError::Transition(TransitionError::WrongPlan), E::WrongPlan),
        (CommandError::Transition(TransitionError::AlreadyDone), E::AlreadyDone),
        (CommandError::Transition(TransitionError::NotASlot), E::NotASlot),
        (
            CommandError::Transition(TransitionError::AlreadyHandled),
            E::AlreadyHandled,
        ),
        (
            CommandError::Transition(TransitionError::NothingToUndo),
            E::NothingToUndo,
        ),
        (
            CommandError::Zone(ZoneError::UnknownZone("A/B".to_owned())),
            E::UnknownZone {
                name: "A/B".to_owned(),
            },
        ),
        (CommandError::Zone(ZoneError::OutOfRange), E::ZoneOutOfRange),
        (CommandError::MissingProgress, E::MissingProgress),
        (
            CommandError::Storage(StorageError::StaleRevision),
            E::Storage {
                reason: StorageError::StaleRevision.to_string(),
            },
        ),
    ];
    for (from, to) in cases {
        assert_eq!(ReminderError::from(from.clone()), to, "for {from:?}");
    }
}

// --- the exported store -------------------------------------------------------

/// 2026-11-18T17:00:00Z, which is noon in New York. The device zone in these
/// tests is New York, in standard time (UTC-5), so 09:00 there is 14:00Z.
const NOW_MS: i64 = 1_795_021_200_000;

fn context(now_ms: i64) -> CallContext {
    CallContext {
        now_ms,
        device_zone: "America/New_York".to_owned(),
    }
}

fn id(n: u128) -> String {
    uuid::Uuid::from_u128(n).to_string()
}

fn ms(text: &str) -> i64 {
    text.parse::<Timestamp>()
        .expect("a valid timestamp")
        .as_millisecond()
}

/// Daily at 09:00 from 2026-11-16, in whatever zone the device is in.
fn daily_plan() -> Plan {
    daily_with(Rule::Daily { every: 1 }, at_nine(), None)
}

fn pills() -> ReminderDraft {
    ReminderDraft {
        title: "Take pills".to_owned(),
        notes: "with water".to_owned(),
        plan: daily_plan(),
    }
}

#[test]
fn the_clock_constant_is_noon_in_new_york() {
    assert_eq!(NOW_MS, ms("2026-11-18T17:00:00Z"));
}

#[test]
fn a_reminder_goes_through_its_whole_life_across_the_boundary() {
    let store = ReminderStore::open_in_memory().expect("opened");
    let c = || context(NOW_MS);
    let reminder_id = new_reminder_id(NOW_MS).expect("an id");

    // Create.
    let created = store.create(c(), reminder_id.clone(), pills()).expect("created");
    assert_eq!(created.reminder.id, reminder_id);
    assert_eq!(created.reminder.title, "Take pills");
    assert_eq!(created.reminder.meta.revision, 1);
    assert!(matches!(
        created.progress.as_ref().map(|progress| &progress.state),
        Some(ProgressState::Cadence { .. })
    ));
    assert_eq!(store.list().expect("listed"), vec![created.clone()]);
    assert_eq!(store.get(reminder_id.clone()).expect("found"), created);

    // The next slot is the overdue one on the 16th.
    let upcoming = store.upcoming(c(), 10).expect("upcoming");
    assert_eq!(upcoming.upcoming.len(), 1);
    assert_eq!(upcoming.upcoming[0].slot, "2026-11-16T09:00:00");
    assert_eq!(upcoming.upcoming[0].at_ms, ms("2026-11-16T14:00:00Z"));
    assert_eq!(upcoming.upcoming[0].title, "Take pills");
    assert!(upcoming.unresolved.is_empty());

    // Resolve it, and see progress move.
    let resolution_id = new_resolution_id(NOW_MS).expect("an id");
    let resolved = store
        .resolve(
            c(),
            reminder_id.clone(),
            resolution_id.clone(),
            Outcome::Done {
                slot: Some("2026-11-16T09:00:00".to_owned()),
            },
            Some("on time".to_owned()),
        )
        .expect("resolved");
    assert_eq!(resolved.resolution.note.as_deref(), Some("on time"));
    let Some(ProgressState::Cadence { current, .. }) =
        resolved.progress.as_ref().map(|progress| progress.state.clone())
    else {
        panic!("expected cadence progress");
    };
    assert_eq!(
        current,
        Watermark {
            through: Some("2026-11-16T09:00:00".to_owned()),
            handled: 1,
        }
    );
    assert_eq!(store.history(reminder_id.clone()).expect("history").len(), 1);
    assert_eq!(
        store.upcoming(c(), 10).expect("upcoming").upcoming[0].slot,
        "2026-11-17T09:00:00"
    );

    // Undo it.
    store
        .undo(c(), reminder_id.clone(), resolution_id)
        .expect("undone");
    assert!(store.history(reminder_id.clone()).expect("history").is_empty());
    assert_eq!(
        store.upcoming(c(), 10).expect("upcoming").upcoming[0].slot,
        "2026-11-16T09:00:00"
    );

    // Edit: a stale revision is refused, the current one accepted.
    let rename = |title: &str| ReminderEdit {
        title: Some(title.to_owned()),
        notes: None,
        plan: None,
        archived: None,
    };
    assert_eq!(
        store.edit(c(), reminder_id.clone(), 9, rename("Vitamins")),
        Err(ReminderError::Conflict { current: 1 })
    );
    let edited = store
        .edit(c(), reminder_id.clone(), 1, rename("Vitamins"))
        .expect("edited");
    assert_eq!(edited.reminder.title, "Vitamins");
    assert_eq!(edited.reminder.meta.revision, 2);

    // Delete.
    store.delete(c(), reminder_id.clone(), 2).expect("deleted");
    assert_eq!(store.get(reminder_id), Err(ReminderError::NotFound));
    assert!(store.list().expect("listed").is_empty());
}

#[test]
fn model_errors_reach_kotlin_as_their_own_cases() {
    let store = ReminderStore::open_in_memory().expect("opened");
    let c = || context(NOW_MS);

    // A fixed zone the database does not know, which only the command layer can check.
    let unknown_zone = ReminderDraft {
        title: "Flight".to_owned(),
        notes: String::new(),
        plan: Plan::Once {
            due: Some(Due {
                at: When {
                    local: "2026-11-20T09:00:00".to_owned(),
                    zone: Zone::Fixed {
                        name: "Not/AZone".to_owned(),
                    },
                },
                alert: alert(),
            }),
        },
    };
    assert_eq!(
        store.create(c(), id(1), unknown_zone),
        Err(ReminderError::UnknownZone {
            name: "Not/AZone".to_owned()
        })
    );

    store.create(c(), id(2), pills()).expect("created");
    assert_eq!(
        store.create(c(), id(2), pills()),
        Err(ReminderError::AlreadyExists)
    );
    assert_eq!(
        store.resolve(
            c(),
            id(2),
            id(10),
            Outcome::Skipped {
                slot: "2026-11-17T10:00:00".to_owned(),
            },
            None,
        ),
        Err(ReminderError::NotASlot)
    );
    assert_eq!(
        store.undo(c(), id(2), id(11)),
        Err(ReminderError::NotFound)
    );
}

#[test]
fn upcoming_reports_what_it_could_not_schedule() {
    // Nothing the FFI can build is unschedulable, so check the mapping directly.
    let missing = UnresolvedReason::from(crate::commands::UnresolvedReason::MissingProgress);
    assert_eq!(missing, UnresolvedReason::MissingProgress);
    let unknown = UnresolvedReason::from(crate::commands::UnresolvedReason::Zone(
        ZoneError::UnknownZone("A/B".to_owned()),
    ));
    assert_eq!(
        unknown,
        UnresolvedReason::UnknownZone {
            name: "A/B".to_owned()
        }
    );
}

#[test]
fn ids_are_version_7_uuid_text_stamped_with_the_given_time() {
    for make in [new_reminder_id, new_resolution_id] {
        let text = make(NOW_MS).expect("an id");
        let parsed = uuid::Uuid::parse_str(&text).expect("UUID text");
        assert_eq!(parsed.get_version_num(), 7);
        let (seconds, _) = parsed.get_timestamp().expect("a v7 id has a time").to_unix();
        assert_eq!(i64::try_from(seconds).expect("fits"), NOW_MS / 1000);
    }
}

#[test]
fn a_weekly_rule_and_its_weekdays_cross_the_boundary() {
    let store = ReminderStore::open_in_memory().expect("opened");
    let draft = ReminderDraft {
        title: "Gym".to_owned(),
        notes: String::new(),
        plan: daily_with(
            Rule::Weekly {
                every: 1,
                on: vec![Weekday::Monday, Weekday::Thursday],
            },
            at_nine(),
            Some(End::Count { count: 3 }),
        ),
    };

    let created = store.create(context(NOW_MS), id(1), draft.clone()).expect("created");

    // 2026-11-16 is a Monday, so the slots are Mon 16th, Thu 19th, Mon 23rd.
    assert_eq!(created.reminder.plan, draft.plan);
    let slots: Vec<_> = store
        .upcoming(context(NOW_MS), 10)
        .expect("upcoming")
        .upcoming
        .iter()
        .map(|upcoming| upcoming.slot.clone())
        .collect();
    assert_eq!(slots, vec!["2026-11-16T09:00:00".to_owned()]);
}

#[test]
fn nth_weekday_rules_cross_the_boundary() {
    let plan = daily_with(
        Rule::Monthly {
            every: 1,
            on: MonthlyOn::NthWeekday {
                nth: Nth::Last,
                weekday: Weekday::Friday,
            },
        },
        at_nine(),
        None,
    );
    let across = domain::Plan::try_from(plan.clone()).expect("a valid plan");
    assert_eq!(Plan::from(across), plan);
}

#[test]
fn a_copied_database_opens_as_a_store_with_the_same_reminders() {
    let directory = std::env::temp_dir();
    let name = |suffix: &str| directory.join(format!("reminder-ffi-test-{}-{suffix}.db", id(7)));
    let (source, destination) = (name("source"), name("copy"));
    for path in [&source, &destination] {
        for extension in ["", "-wal", "-shm"] {
            let mut file = path.clone().into_os_string();
            file.push(extension);
            let _ = std::fs::remove_file(file);
        }
    }

    let store = ReminderStore::open(source.to_string_lossy().into_owned()).expect("opened");
    let created = store
        .create(context(NOW_MS), id(1), pills())
        .expect("created");

    super::copy_database(
        source.to_string_lossy().into_owned(),
        destination.to_string_lossy().into_owned(),
    )
    .expect("copied");
    let copy = ReminderStore::open(destination.to_string_lossy().into_owned()).expect("opened");
    assert_eq!(copy.get(id(1)).expect("in the copy"), created);

    drop((store, copy));
    for path in [&source, &destination] {
        for extension in ["", "-wal", "-shm"] {
            let mut file = path.clone().into_os_string();
            file.push(extension);
            let _ = std::fs::remove_file(file);
        }
    }
}
