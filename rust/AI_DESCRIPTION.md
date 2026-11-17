# Description of `reminder-core`

**An AI (Claude) wrote this file during development, as a plain description of what the Rust core contains.** It can be out of date or wrong. The code is the source of truth.

## What it is

`rust/core` is the `reminder-core` crate: the domain model and logic for a personal reminder app, exposed to Kotlin through UniFFI. `ping()` and a `ReminderStore` object are exported. No Kotlin code calls the store yet, and nothing opens a database in the app.

The core never reads the clock or the device time zone. Callers pass `now` and the device zone in.

## Modules (`rust/core/src`)

| Module | Content |
|---|---|
| `model` | The data types. |
| `model/values.rs` | Validated value types, built only through `new`. |
| `recurrence` | `next_slot`: the next occurrence of a recurrence. |
| `progress` | Pure state transitions on `Progress`. |
| `instant` | `instant_of`: turns a local time slot into an instant. |
| `commands` | The command layer: the `Reminders` trait, `LocalReminders` and the `Storage` port. |
| `store` | `SqliteStorage`: the SQLite implementation of the `Storage` port. |
| `ffi` | What Kotlin sees: mirror types, `ReminderError` and the `ReminderStore` object. |

Each module keeps its tests in a `tests.rs` beside it.

## The model

Three kinds of data:

- **Entities** are mutable and synced. A `Reminder` is what the user defined. A `Progress` is how far its plan has advanced. They are separate so that editing and resolving on different devices do not overwrite each other. Both carry a revision.
- **Events.** A `Resolution` is an append-only archive record of a response (done or skipped). It is written alongside the `Progress` change it causes and is read only by history and statistics, never by scheduling or completion logic.
- **Derived or device-local state** (next fire time, scheduled alarms, snooze, delivered notifications) is not part of the model.

### Reminder

A `Reminder` has an id, a `Title`, notes, a `Plan`, an optional `archived_at` and a `Meta` (revision, created, updated, deleted tombstone).

A `Plan` is one of:

- `Once { due: Option<Due> }`: finished by one completion. `Due` is a `When` (local date and time plus a `Zone`) and an `Alert`.
- `Ongoing { cadence: Option<Cadence> }`: never finishes. `Cadence` is a `Recurrence` and an `Alert`. With no cadence it is a tracker whose completions are only logged.

Only a scheduled plan carries an `Alert`. An alert has a style, `Notification { sticky }` or `Alarm`, and an optional `Nudge` that re-alerts at an interval, with an optional limit.

A `Zone` is `Floating` (follows the device) or `Fixed(name)` (an IANA name, checked when a plan is created or edited and again when it is resolved).

### Recurrence

A `Recurrence` has `starts_on` (a date), a `Zone`, a `Rule`, `TimesOfDay` and an optional `End`.

- `Rule` selects the days: `Daily`, `Weekly` (a set of weekdays), `Monthly` (a day of the month, or the nth weekday: first to fourth, or last) and `Yearly`. Each has an `every` count.
- `TimesOfDay` selects times within each day: `At` a set of clock times, or `Every` step inside a window of the day.
- `End` is `Until` a local date and time, or `Count` of slots.

A `Slot` is a nominal local date and time. It is how an occurrence is identified. It is not an instant.

### Progress

`Progress` has the reminder id, a `ProgressState`, `last_resolution`, a revision and an update time. The state is `Once { done_at }` or `Cadence { current, previous }`. A tracker has none.

A `Watermark` is how far a cadence has been handled: `through` (the last handled slot) and `handled` (the number of slots up to it, used for `End::Count`). Slots up to the watermark are handled, whether done, skipped or missed. `previous` holds the watermark before the last resolution, for a one-level undo. `last_resolution` is the id of the resolution that produced the current state, so exactly that one can be undone. It is cleared by an undo.

### Identifiers

`ReminderId` and `ResolutionId` wrap a `Uuid`. `new(now)` makes a UUIDv7 stamped with `now` to the millisecond. Instants before the Unix epoch are stamped as the epoch.

### Validated values (`model/values.rs`)

Private fields, built through `new`, which returns `InvalidValue` on bad input: `Weekdays` (non-empty set), `DayOfMonth` (1 to 31), `PositiveDuration`, `ClockTimes` (non-empty, sorted, no duplicates), `Window` (start not after end) and `Title` (not blank, trimmed). Counts that cannot be zero use `NonZeroU32`.

## Logic

### `recurrence::next_slot(recurrence, watermark) -> Option<Slot>`

The first slot strictly after the watermark, or `None` when the recurrence has ended or the calendar runs out. It works in civil time only.

- `Daily`, `Monthly` and `Yearly` count `every` days, months or years from the day, month or year of `starts_on`. `Weekly` counts weeks that start on Monday, from the week containing `starts_on`.
- A monthly day, or a yearly anchor day, past the end of a short month clamps to that month's last day. A yearly February 29 becomes February 28 in a common year.
- `Until` is inclusive. `Count` counts slots, not days.
- Slots inside an `Every` window are computed by division, not by stepping.

### `progress` transitions

All are pure and take `now`:

- `Progress::initial(id, plan, now)`: `None` for a tracker.
- `resolve(plan, slot, resolution, now)`: completes a one-off. For a cadence, moves the watermark to a later slot and counts the slots it passes over. Errors: `WrongPlan`, `AlreadyDone`, `NotASlot`, `AlreadyHandled`.
- `undo(now)`: one level deep. Error: `NothingToUndo`.
- `next_pending(plan)`: a one-off's due time while open, or the next slot after a cadence's watermark.

Each transition increments the revision. `resolve` does not take an `Outcome`, because done and skipped advance progress the same way. It does not check whether the slot is in the future. The command layer does.

### `instant::instant_of(slot, zone, device) -> Result<Timestamp, ZoneError>`

`Floating` uses the device zone passed in. `Fixed` looks the name up in the time zone database. Errors: `UnknownZone`, `OutOfRange`.

- A local time that happens twice (clocks go back) resolves to the first occurrence.
- A local time that never happens (clocks go forward) uses the offset from before the gap, so 02:30 in a 02:00 to 03:00 gap fires at 03:30.
- Resolving slots in order does not always give instants in order, and two slots can resolve to the same instant.

## The command layer (`commands`)

The API that the FFI, and later a network client, are meant to wrap.

```
Reminders (trait)
  └─ LocalReminders<S: Storage>   validation and orchestration over a Storage
       └─ Storage (trait)         persistence port: SqliteStorage, plus an in-memory one for tests
```

Every command takes a `Context { now, device_zone }`. Identifiers come from the caller. A reused id fails with `CommandError::AlreadyExists`.

### Commands

| Command | Behaviour |
|---|---|
| `create(ctx, id, draft)` | Builds the reminder at revision 1 and its initial `Progress`. Validates the plan. |
| `edit(ctx, id, base_revision, edit)` | Applies title, notes, plan and archive changes if the revision matches, else `Conflict { current }`. Replacing a plan keeps `Progress` when the plan stays the same kind (one-off, cadence, tracker) and resets it, with a higher revision, when the kind changes. An archived reminder accepts only `archived: Some(false)`. |
| `delete(ctx, id, base_revision)` | Tombstones the reminder and drops its progress. Its resolutions stay. |
| `resolve(ctx, id, resolution_id, outcome, note)` | See the table below. |
| `undo(ctx, id, resolution_id)` | The resolution that is `Progress::last_resolution` rolls progress back and is retracted. A resolution that moved nothing (no slot) is just retracted. Any other is `NotLatest`. |
| `get`, `list` | A reminder with its progress. `list` is oldest first and leaves out deleted ones. |
| `upcoming(ctx, limit)` | Each unarchived, undeleted reminder's pending slot, resolved to an instant and sorted by instant. Overdue slots are included. Reminders whose zone cannot be resolved, or that lack a progress row, are returned in `unresolved`. |
| `alarms(ctx, limit)` | For each unarchived, undeleted reminder, the next slot that has not fired yet, resolved to an instant and sorted by instant. A slot that already passed unresolved is skipped, not repeated, so an ignored slot never hides the ones after it. A one-off only counts while its due time is ahead. Unresolvable reminders are returned in `unresolved`. This is what to set alarms from. |
| `history(id)` | A reminder's resolutions, newest first, without retracted ones. |

`resolve` by plan:

| Plan | Outcome | Effect |
|---|---|---|
| one-off | `Done` with no slot | completes it |
| cadence | `Done` or `Skipped` with a slot | moves the watermark |
| cadence | `Done` with no slot | log only |
| tracker | `Done` with no slot | log only |

Anything else is `Invalid(OutcomeNotAllowed)`. A slot may be resolved if it is the next pending slot, even before it fires, or if it has already fired. Otherwise it is `NotResolvable`.

### Validation

A plan is rejected if a `Fixed` zone name is unknown to the time zone database or an `Until` is earlier than `starts_on`. Archived reminders cannot be resolved or undone.

### Storage port

`Storage` reads reminders, progress and resolutions, and takes writes as one `WriteSet` (an optional reminder, progress and resolution write) that must apply atomically. Each reminder or progress put carries an `Expect`: no row, or a row at a given revision. A resolution is inserted (its id must be new) or updated (it must exist). Errors: `AlreadyExists`, `Missing`, `StaleRevision`, `Corrupt`, `Backend`.

`LocalReminders` is the only caller. `MemoryStorage` implements the port for tests and is compiled only under `cfg(test)`. Both storages share `check_expect`, the rule for comparing a row's revision with what a write expects.

## The SQLite store (`store`)

`SqliteStorage::open(path)` or `open_in_memory()` opens a database through rusqlite on plain SQL, with WAL, foreign keys on and `synchronous = NORMAL`. Opening applies any missing migrations. A database from a newer app is refused.

- **Migrations** are Rust strings in `store/migrations.rs`, applied in order and tracked in `PRAGMA user_version`. They are strings, not `.sql` files, so builds that copy only Rust sources still see them.
- **Tables.** `reminders`, `progress` and `resolutions`, all `STRICT`. Ids are 16-byte blobs. Timestamps are Unix milliseconds, so anything finer is dropped. Local dates and times are ISO 8601 text.
- **The plan** is one JSON text column on `reminders`, written and read through serde. Serde is derived on the plan types. The validated types deserialize through their `new` constructors, so stored data that breaks a model rule fails to load.
- **Progress** has a `kind` of `once` or `cadence` and typed columns for the watermark and its previous value. **Resolutions** have an outcome of `done` or `skipped` and an optional slot. CHECK constraints tie the columns to the kind or outcome, and foreign keys tie progress and resolutions to their reminder.
- **Commits** run in one immediate transaction. Every expectation is checked before anything is written, so a refused write set changes nothing. Errors match `MemoryStorage`'s.
- **Corrupt data.** A row that cannot be turned back into a valid model value is reported as `StorageError::Corrupt`.
- **Copying.** `copy_database(source, destination)` writes a consistent copy of a database with `VACUUM INTO`, so it includes what is still in the write-ahead log, which copying the file would miss. It opens the source as it is: no migration, and no file is created if the source is missing. The destination must not exist.

## The FFI layer (`ffi`)

`ReminderStore` is the one exported object. It owns a `LocalReminders<SqliteStorage>` behind a mutex, so calls are serialized. Calls block, so Kotlin makes them off the UI thread.

- **Constructors:** `open(path)` and `open_in_memory()`.
- **Methods:** `create`, `edit`, `delete`, `resolve`, `undo`, `get`, `list`, `upcoming`, `alarms` and `history`, with the same meaning as the commands they wrap.
- **Free functions:** `new_reminder_id(now_ms)` and `new_resolution_id(now_ms)` return UUIDv7 text, so Kotlin never invents ids. `copy_database(source, destination)` is the store's copy, for keeping one database file per app version.
- **Context:** commands that need the time take a `CallContext { now_ms, device_zone }` on every call. `device_zone` is an IANA zone name. An unknown one is `InvalidInput`. (The model's `Context` is renamed because Android has its own.)
- **Mirror types** (`ffi/types.rs`) copy the model one to one using only what UniFFI carries. Instants are Unix milliseconds, ids are UUID text, local dates and times are ISO 8601 text, durations are `std::time::Duration`, and weekdays are an enum. `Due.when` is `Due.at` because `when` is a Kotlin keyword.
- **Input** is turned into model values through the model's constructors. Anything the model would refuse comes back as `ReminderError::InvalidInput` with the reason.
- **`ReminderError`** is a flat enum with a case for each command failure (`NotFound`, `Conflict { current }`, `AlreadyHandled`, `UnknownZone { name }` and so on), plus `InvalidInput` and `Storage`. Kotlin gets one exception type with a subclass per case.

## Tests

`proptest` property tests plus pinned examples. The recurrence tests compare `next_slot` with a brute-force day-by-day enumeration written a different way. The DST tests use a POSIX rule string for US Eastern time; two tests look up `America/New_York` in the machine's time zone database. The command tests run against `MemoryStorage`, and one property test runs random command sequences and checks that progress still matches each plan, revisions never decrease, and `upcoming` stays sorted and excludes archived and completed reminders. The store tests run the same storage-rule checks against both storages, check the schema's constraints and corrupt-data handling, reopen a database file, and run random command sequences on `MemoryStorage` and `SqliteStorage` side by side, comparing every result and state. Serde tests check that the validated types refuse JSON they would not build from and that values survive a round trip.

The FFI tests check that plans and outcomes survive a round trip through the mirror types, that input the model would refuse is refused, the `CommandError` to `ReminderError` mapping case by case, and a reminder's whole life through the exported store using its Rust API.

## Dependencies

`jiff` 1.x (dates, times, zones, with its `serde` feature), `uuid` (v7), `rusqlite` (bundled SQLite), `serde` and `serde_json` (the plan column), `uniffi`. `proptest` for tests.
