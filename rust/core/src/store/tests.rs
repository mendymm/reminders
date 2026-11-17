//! Tests for [`SqliteStorage`]: the same storage rules the in-memory test storage
//! follows, the database's own constraints, corrupt data, persistence on disk,
//! and a comparison of the two storages under random command sequences.

use std::num::NonZeroU32;
use std::path::PathBuf;

use jiff::{SignedDuration, Timestamp, ToSpan, civil};
use proptest::prelude::*;
use rusqlite::{Connection, params};
use uuid::Uuid;

use super::codec::id_bytes;
use super::migrations::MIGRATIONS;
use super::SqliteStorage;
use crate::commands::memory::MemoryStorage;
use crate::commands::tests::{Driver, NOW, any_op, apply, ctx, seed};
use crate::commands::{
    Expect, LocalReminders, ProgressWrite, ReminderDraft, Reminders, ResolutionWrite, Storage,
    StorageError, WriteSet,
};
use crate::model::{
    Alert, AlertStyle, Cadence, Due, Meta, MonthlyOn, Nudge, Outcome, Plan, PositiveDuration,
    Progress, ProgressState, Reminder, ReminderId, Resolution, ResolutionId, Rule, Slot, Title,
    When, Zone,
};
use crate::recurrence::tests::any_recurrence;

fn rid(n: u128) -> ReminderId {
    ReminderId(Uuid::from_u128(n))
}

fn xid(n: u128) -> ResolutionId {
    ResolutionId(Uuid::from_u128(n))
}

fn at_ms(milliseconds: i64) -> Timestamp {
    Timestamp::from_millisecond(milliseconds).expect("in range")
}

fn reminder(id: u128, revision: u64) -> Reminder {
    Reminder {
        id: rid(id),
        title: Title::new("A reminder").expect("not blank"),
        notes: "some notes".to_owned(),
        plan: Plan::Ongoing { cadence: None },
        archived_at: None,
        meta: Meta {
            revision,
            created_at: at_ms(1_000),
            updated_at: at_ms(2_000),
            deleted_at: None,
        },
    }
}

fn progress(id: u128, revision: u64) -> Progress {
    Progress {
        reminder_id: rid(id),
        state: ProgressState::Once { done_at: None },
        last_resolution: None,
        revision,
        updated_at: at_ms(0),
    }
}

fn resolution(id: u128, reminder: u128, resolved_at: i64) -> Resolution {
    Resolution {
        id: xid(id),
        reminder_id: rid(reminder),
        resolved_at: at_ms(resolved_at),
        outcome: Outcome::Done { slot: None },
        note: None,
        deleted_at: None,
    }
}

fn put_reminder(reminder: Reminder, expect: Expect) -> WriteSet {
    WriteSet {
        reminder: Some((reminder, expect)),
        ..WriteSet::default()
    }
}

fn put_progress(progress: Progress, expect: Expect) -> WriteSet {
    WriteSet {
        progress: Some(ProgressWrite::Put(progress, expect)),
        ..WriteSet::default()
    }
}

fn write_resolution(write: ResolutionWrite) -> WriteSet {
    WriteSet {
        resolution: Some(write),
        ..WriteSet::default()
    }
}

/// The rules every `Storage` must follow.
fn exercise<S: Storage>(storage: &mut S) {
    // Reminders: created once, then updated only from the revision read.
    storage
        .commit(put_reminder(reminder(1, 1), Expect::Absent))
        .expect("created");
    assert_eq!(
        storage.commit(put_reminder(reminder(1, 1), Expect::Absent)),
        Err(StorageError::AlreadyExists)
    );
    storage
        .commit(put_reminder(reminder(1, 2), Expect::Revision(1)))
        .expect("updated");
    assert_eq!(
        storage.commit(put_reminder(reminder(1, 3), Expect::Revision(1))),
        Err(StorageError::StaleRevision)
    );
    assert_eq!(
        storage.commit(put_reminder(reminder(9, 2), Expect::Revision(1))),
        Err(StorageError::StaleRevision)
    );
    assert_eq!(storage.reminder(rid(1)).expect("read"), Some(reminder(1, 2)));
    assert_eq!(storage.reminder(rid(9)).expect("read"), None);

    // Progress: the same rules, and it can be deleted.
    storage
        .commit(put_progress(progress(1, 1), Expect::Absent))
        .expect("created");
    assert_eq!(
        storage.commit(put_progress(progress(1, 1), Expect::Absent)),
        Err(StorageError::AlreadyExists)
    );
    assert_eq!(
        storage.commit(put_progress(progress(1, 2), Expect::Revision(7))),
        Err(StorageError::StaleRevision)
    );
    storage
        .commit(put_progress(progress(1, 2), Expect::Revision(1)))
        .expect("updated");
    assert_eq!(storage.progress(rid(1)).expect("read"), Some(progress(1, 2)));
    let delete = WriteSet {
        progress: Some(ProgressWrite::Delete(rid(1))),
        ..WriteSet::default()
    };
    storage.commit(delete.clone()).expect("deleted");
    assert_eq!(storage.progress(rid(1)).expect("read"), None);
    storage.commit(delete).expect("deleting nothing is fine");

    // Resolutions: inserted once, updated only when present.
    storage
        .commit(write_resolution(ResolutionWrite::Insert(resolution(10, 1, 1_000))))
        .expect("inserted");
    assert_eq!(
        storage.commit(write_resolution(ResolutionWrite::Insert(resolution(10, 1, 5_000)))),
        Err(StorageError::AlreadyExists)
    );
    let mut retracted = resolution(10, 1, 1_000);
    retracted.deleted_at = Some(at_ms(9_000));
    storage
        .commit(write_resolution(ResolutionWrite::Update(retracted.clone())))
        .expect("updated");
    assert_eq!(storage.resolution(xid(10)).expect("read"), Some(retracted));
    assert_eq!(
        storage.commit(write_resolution(ResolutionWrite::Update(resolution(77, 1, 0)))),
        Err(StorageError::Missing)
    );
    assert_eq!(storage.resolution(xid(77)).expect("read"), None);

    // A refused write set changes nothing, whichever part is refused.
    let before = storage.reminder(rid(1)).expect("read");
    let stale_progress = WriteSet {
        reminder: Some((reminder(1, 3), Expect::Revision(2))),
        progress: Some(ProgressWrite::Put(progress(1, 5), Expect::Revision(99))),
        resolution: None,
    };
    assert_eq!(storage.commit(stale_progress), Err(StorageError::StaleRevision));
    assert_eq!(storage.reminder(rid(1)).expect("read"), before);
    let duplicate_resolution = WriteSet {
        reminder: Some((reminder(1, 3), Expect::Revision(2))),
        progress: None,
        resolution: Some(ResolutionWrite::Insert(resolution(10, 1, 5_000))),
    };
    assert_eq!(
        storage.commit(duplicate_resolution),
        Err(StorageError::AlreadyExists)
    );
    assert_eq!(storage.reminder(rid(1)).expect("read"), before);

    // A write set applies as a whole.
    let together = WriteSet {
        reminder: Some((reminder(1, 3), Expect::Revision(2))),
        progress: Some(ProgressWrite::Put(progress(1, 1), Expect::Absent)),
        resolution: Some(ResolutionWrite::Insert(resolution(11, 1, 3_000))),
    };
    storage.commit(together).expect("committed together");
    assert_eq!(storage.reminder(rid(1)).expect("read"), Some(reminder(1, 3)));
    assert_eq!(storage.progress(rid(1)).expect("read"), Some(progress(1, 1)));

    // Resolutions come back newest first, ties broken by id, retracted included.
    for (id, resolved_at) in [(12, 2_000), (13, 3_000), (14, 3_000)] {
        storage
            .commit(write_resolution(ResolutionWrite::Insert(resolution(
                id,
                1,
                resolved_at,
            ))))
            .expect("inserted");
    }
    let order: Vec<_> = storage
        .resolutions(rid(1))
        .expect("read")
        .iter()
        .map(|resolution| resolution.id)
        .collect();
    assert_eq!(order, vec![xid(14), xid(13), xid(11), xid(12), xid(10)]);

    // Deleted reminders are still listed. Telling them apart is the caller's job.
    let mut gone = reminder(2, 1);
    gone.meta.deleted_at = Some(at_ms(5_000));
    storage
        .commit(put_reminder(gone, Expect::Absent))
        .expect("created");
    assert_eq!(storage.reminders().expect("read").len(), 2);
}

#[test]
fn the_in_memory_test_storage_follows_the_storage_rules() {
    exercise(&mut MemoryStorage::default());
}

#[test]
fn sqlite_follows_the_storage_rules() {
    exercise(&mut SqliteStorage::open_in_memory().expect("opened"));
}

#[test]
fn timestamps_are_stored_to_the_millisecond() {
    let mut storage = SqliteStorage::open_in_memory().expect("opened");
    let mut precise = reminder(1, 1);
    precise.meta.created_at = Timestamp::new(1_800_000_000, 123_456_789).expect("in range");
    storage
        .commit(put_reminder(precise.clone(), Expect::Absent))
        .expect("created");

    let read = storage.reminder(rid(1)).expect("read").expect("stored");
    assert_eq!(
        read.meta.created_at.as_millisecond(),
        precise.meta.created_at.as_millisecond()
    );
    assert_eq!(read.meta.created_at.subsec_nanosecond(), 123_000_000);
}

// --- the database's own constraints -----------------------------------------

fn storage_with_a_reminder() -> SqliteStorage {
    let mut storage = SqliteStorage::open_in_memory().expect("opened");
    storage
        .commit(put_reminder(reminder(1, 1), Expect::Absent))
        .expect("created");
    storage
}

#[test]
fn the_schema_refuses_a_skip_without_a_slot() {
    let storage = storage_with_a_reminder();
    let result = storage.conn.execute(
        "INSERT INTO resolutions (id, reminder_id, resolved_at, outcome) \
         VALUES (?1, ?2, 0, 'skipped')",
        params![id_bytes(Uuid::from_u128(10)), id_bytes(Uuid::from_u128(1))],
    );
    assert!(result.is_err());
}

#[test]
fn the_schema_refuses_progress_whose_columns_disagree_with_its_kind() {
    let storage = storage_with_a_reminder();
    let result = storage.conn.execute(
        "INSERT INTO progress (reminder_id, kind, through, revision, updated_at) \
         VALUES (?1, 'once', '2026-11-16T09:00:00', 1, 0)",
        params![id_bytes(Uuid::from_u128(1))],
    );
    assert!(result.is_err());
}

#[test]
fn the_schema_refuses_rows_for_a_reminder_that_does_not_exist() {
    let storage = storage_with_a_reminder();
    let result = storage.conn.execute(
        "INSERT INTO progress (reminder_id, kind, done_at, revision, updated_at) \
         VALUES (?1, 'once', NULL, 1, 0)",
        params![id_bytes(Uuid::from_u128(99))],
    );
    assert!(result.is_err());
}

// --- corrupt data --------------------------------------------------------------

fn insert_reminder_with_plan(storage: &SqliteStorage, plan: &str) {
    storage
        .conn
        .execute(
            "INSERT INTO reminders (id, title, notes, plan, revision, created_at, updated_at) \
             VALUES (?1, 'Broken', '', ?2, 1, 0, 0)",
            params![id_bytes(Uuid::from_u128(1)), plan],
        )
        .expect("inserted");
}

#[test]
fn a_plan_that_is_not_json_is_reported_as_corrupt() {
    let storage = SqliteStorage::open_in_memory().expect("opened");
    insert_reminder_with_plan(&storage, "not json");

    assert!(matches!(
        storage.reminder(rid(1)),
        Err(StorageError::Corrupt(_))
    ));
}

#[test]
fn a_plan_that_breaks_a_model_rule_is_reported_as_corrupt() {
    // Well-formed JSON for a plan, but with a day of the month of 40.
    let valid = Plan::Ongoing {
        cadence: Some(Cadence {
            recurrence: crate::model::Recurrence {
                starts_on: civil::date(2026, 11, 16),
                zone: Zone::Floating,
                rule: Rule::Monthly {
                    every: NonZeroU32::new(1).expect("one"),
                    on: MonthlyOn::Day(crate::model::DayOfMonth::new(15).expect("1 to 31")),
                },
                at: crate::model::TimesOfDay::At(
                    crate::model::ClockTimes::new([civil::time(9, 0, 0, 0)]).expect("one time"),
                ),
                end: None,
            },
            alert: Alert {
                style: AlertStyle::Alarm,
                nudge: None,
            },
        }),
    };
    let mut json = serde_json::to_value(&valid).expect("serialized");
    *json
        .pointer_mut("/Ongoing/cadence/recurrence/rule/Monthly/on/Day")
        .expect("the day of the month is where the JSON puts it") = serde_json::json!(40);

    let storage = SqliteStorage::open_in_memory().expect("opened");
    insert_reminder_with_plan(&storage, &json.to_string());

    assert!(matches!(
        storage.reminder(rid(1)),
        Err(StorageError::Corrupt(_))
    ));
}

// --- persistence -----------------------------------------------------------------

/// A database file in the temp directory, removed on drop.
struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("reminder-core-test-{}.db", Uuid::now_v7())))
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.0.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(path);
        }
    }
}

#[test]
fn data_survives_closing_and_reopening_the_database() {
    let database = TempDatabase::new();
    let c = ctx(NOW);
    let created = {
        let mut service =
            LocalReminders::new(SqliteStorage::open(&database.0).expect("opened"));
        let draft = ReminderDraft {
            title: Title::new("Persistent").expect("not blank"),
            notes: "kept".to_owned(),
            plan: Plan::Once { due: None },
        };
        service.create(&c, rid(1), draft).expect("created")
    };

    let service = LocalReminders::new(SqliteStorage::open(&database.0).expect("reopened"));
    assert_eq!(service.get(rid(1)).expect("still there"), created);
}

#[test]
fn opening_applies_every_migration_and_records_the_version() {
    let storage = SqliteStorage::open_in_memory().expect("opened");
    let version: i64 = storage
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read");
    assert_eq!(version, i64::try_from(MIGRATIONS.len()).expect("a small count"));
}

#[test]
fn a_database_from_a_newer_app_is_refused() {
    let database = TempDatabase::new();
    {
        let conn = Connection::open(&database.0).expect("opened");
        conn.execute_batch(&format!("PRAGMA user_version = {}", MIGRATIONS.len() + 1))
            .expect("set");
    }

    assert!(matches!(
        SqliteStorage::open(&database.0),
        Err(StorageError::Backend(_))
    ));
}

// --- plans and the two storages ---------------------------------------------

fn any_datetime() -> impl Strategy<Value = civil::DateTime> {
    (0..4000i32, 0..86_400_000_000_000i64).prop_map(|(days, nanos)| {
        (civil::date(2020, 1, 1) + days.days())
            .to_datetime(civil::Time::midnight() + SignedDuration::from_nanos(nanos))
    })
}

fn any_zone() -> impl Strategy<Value = Zone> {
    prop_oneof![
        Just(Zone::Floating),
        "[A-Za-z_/]{1,20}".prop_map(Zone::Fixed),
    ]
}

fn any_alert() -> impl Strategy<Value = Alert> {
    let style = prop_oneof![
        Just(AlertStyle::Alarm),
        any::<bool>().prop_map(|sticky| AlertStyle::Notification { sticky }),
    ];
    let nudge = prop::option::of((1..100_000i64, 0..1_000_000_000i32, prop::option::of(1..50u32)))
        .prop_map(|nudge| {
            nudge.map(|(seconds, nanos, limit)| Nudge {
                every: PositiveDuration::new(SignedDuration::new(seconds, nanos))
                    .expect("positive"),
                limit: limit.and_then(NonZeroU32::new),
            })
        });
    (style, nudge).prop_map(|(style, nudge)| Alert { style, nudge })
}

fn any_plan() -> impl Strategy<Value = Plan> {
    prop_oneof![
        Just(Plan::Once { due: None }),
        (any_datetime(), any_zone(), any_alert()).prop_map(|(local, zone, alert)| Plan::Once {
            due: Some(Due {
                when: When { local, zone },
                alert,
            }),
        }),
        Just(Plan::Ongoing { cadence: None }),
        (any_recurrence(), any_alert()).prop_map(|(recurrence, alert)| Plan::Ongoing {
            cadence: Some(Cadence { recurrence, alert }),
        }),
    ]
}

fn any_slot() -> impl Strategy<Value = Slot> {
    any_datetime().prop_map(Slot)
}

proptest! {
    #[test]
    fn a_plan_survives_a_round_trip_through_sqlite(plan in any_plan()) {
        let mut storage = SqliteStorage::open_in_memory().expect("opened");
        let mut stored = reminder(1, 1);
        stored.plan = plan;

        storage.commit(put_reminder(stored.clone(), Expect::Absent)).expect("created");

        prop_assert_eq!(storage.reminder(rid(1)).expect("read"), Some(stored));
    }

    #[test]
    fn progress_and_resolutions_survive_a_round_trip_through_sqlite(
        through in prop::option::of(any_slot()),
        previous in prop::option::of(prop::option::of(any_slot())),
        handled in 0..1_000u32,
        skip_slot in any_slot(),
        note in prop::option::of("[ -~]{0,20}"),
    ) {
        use crate::model::Watermark;
        let mut storage = SqliteStorage::open_in_memory().expect("opened");
        storage.commit(put_reminder(reminder(1, 1), Expect::Absent)).expect("created");

        let cadence = Progress {
            reminder_id: rid(1),
            state: ProgressState::Cadence {
                current: Watermark { through, handled },
                previous: previous.map(|through| Watermark { through, handled: handled / 2 }),
            },
            last_resolution: Some(xid(5)),
            revision: 3,
            updated_at: at_ms(7_000),
        };
        storage.commit(put_progress(cadence.clone(), Expect::Absent)).expect("stored");
        prop_assert_eq!(storage.progress(rid(1)).expect("read"), Some(cadence));

        let skipped = Resolution {
            outcome: Outcome::Skipped { slot: skip_slot },
            note,
            ..resolution(10, 1, 4_000)
        };
        storage
            .commit(write_resolution(ResolutionWrite::Insert(skipped.clone())))
            .expect("stored");
        prop_assert_eq!(storage.resolution(xid(10)).expect("read"), Some(skipped));
    }

    #[test]
    fn sqlite_behaves_like_the_in_memory_storage_under_random_commands(
        ops in prop::collection::vec(any_op(), 1..40),
    ) {
        let c = ctx(NOW);
        let mut memory = LocalReminders::new(MemoryStorage::default());
        let mut sqlite =
            LocalReminders::new(SqliteStorage::open_in_memory().expect("opened"));
        seed(&mut memory, &c);
        seed(&mut sqlite, &c);
        let mut memory_driver = Driver::new();
        let mut sqlite_driver = Driver::new();

        for op in ops {
            let from_memory = apply(&mut memory, &c, &mut memory_driver, op.clone());
            let from_sqlite = apply(&mut sqlite, &c, &mut sqlite_driver, op);
            prop_assert_eq!(from_memory, from_sqlite);
            prop_assert_eq!(memory.list(), sqlite.list());
            prop_assert_eq!(memory.upcoming(&c, 100), sqlite.upcoming(&c, 100));
            for id in crate::commands::tests::ids() {
                prop_assert_eq!(memory.history(id), sqlite.history(id));
            }
        }
    }
}
