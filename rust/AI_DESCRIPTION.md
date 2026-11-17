# Description of `reminder-core`

**An AI (Claude) wrote this file during development, as a plain description of what the Rust core contains.** It can be out of date or wrong. The code is the source of truth.

## What it is

`rust/core` is the `reminder-core` crate: the domain model and logic for a personal reminder app, built to be exposed to Kotlin through UniFFI. Today only `ping()` is exported. Nothing else is wired to UniFFI, to SQLite or to Android yet.

The core never reads the clock or the device time zone. Callers pass `now` and the device zone in.

## Modules (`rust/core/src`)

| Module | Content |
|---|---|
| `model` | The data types. |
| `model/values.rs` | Validated value types, built only through `new`. |
| `recurrence` | `next_slot`: the next occurrence of a recurrence. |
| `progress` | Pure state transitions on `Progress`. |
| `instant` | `instant_of`: turns a local time slot into an instant. |

Each module keeps its tests in a `tests.rs` beside it.

## The model

Three kinds of data:

- **Entities** are mutable and synced. A `Reminder` is what the user defined. A `Progress` is how far its plan has advanced. They are separate so that editing and resolving on different devices do not overwrite each other. Both carry a revision.
- **Events.** A `Resolution` is an append-only archive record of a response (done or skipped). It is written alongside the `Progress` change it causes and is read only by history and statistics, never by scheduling or completion logic.
- **Derived or device-local state** (next fire time, scheduled alarms, snooze, delivered notifications) is not part of the model.

### Reminder

A `Reminder` has an id, title, notes, a `Plan`, an optional `archived_at` and a `Meta` (revision, created, updated, deleted tombstone).

A `Plan` is one of:

- `Once { due: Option<Due> }`: finished by one completion. `Due` is a `When` (local date and time plus a `Zone`) and an `Alert`.
- `Ongoing { cadence: Option<Cadence> }`: never finishes. `Cadence` is a `Recurrence` and an `Alert`. With no cadence it is a tracker whose completions are only logged.

Only a scheduled plan carries an `Alert`. An alert has a style, `Notification { sticky }` or `Alarm`, and an optional `Nudge` that re-alerts at an interval, with an optional limit.

A `Zone` is `Floating` (follows the device) or `Fixed(name)` (an IANA name, not validated until it is resolved).

### Recurrence

A `Recurrence` has `starts_on` (a date), a `Zone`, a `Rule`, `TimesOfDay` and an optional `End`.

- `Rule` selects the days: `Daily`, `Weekly` (a set of weekdays), `Monthly` (a day of the month, or the nth weekday: first to fourth, or last) and `Yearly`. Each has an `every` count.
- `TimesOfDay` selects times within each day: `At` a set of clock times, or `Every` step inside a window of the day.
- `End` is `Until` a local date and time, or `Count` of slots.

A `Slot` is a nominal local date and time. It is how an occurrence is identified. It is not an instant.

### Progress

`Progress` has the reminder id, a `ProgressState`, a revision and an update time. The state is `Once { done_at }` or `Cadence { current, previous }`. A tracker has none.

A `Watermark` is how far a cadence has been handled: `through` (the last handled slot) and `handled` (the number of slots up to it, used for `End::Count`). Slots up to the watermark are handled, whether done, skipped or missed. `previous` holds the watermark before the last resolution, for a one-level undo.

### Identifiers

`ReminderId` and `ResolutionId` wrap a `Uuid`. `new(now)` makes a UUIDv7 stamped with `now` to the millisecond. Instants before the Unix epoch are stamped as the epoch.

### Validated values (`model/values.rs`)

Private fields, built through `new`, which returns `InvalidValue` on bad input: `Weekdays` (non-empty set), `DayOfMonth` (1 to 31), `PositiveDuration`, `ClockTimes` (non-empty, sorted, no duplicates), `Window` (start not after end). Counts that cannot be zero use `NonZeroU32`.

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
- `resolve(plan, slot, now)`: completes a one-off. For a cadence, moves the watermark to a later slot and counts the slots it passes over. Errors: `WrongPlan`, `AlreadyDone`, `NotASlot`, `AlreadyHandled`.
- `undo(now)`: one level deep. Error: `NothingToUndo`.
- `next_pending(plan)`: a one-off's due time while open, or the next slot after a cadence's watermark.

Each transition increments the revision. `resolve` does not take an `Outcome`, because done and skipped advance progress the same way. It does not reject slots in the future, since it has no local "now".

### `instant::instant_of(slot, zone, device) -> Result<Timestamp, ZoneError>`

`Floating` uses the device zone passed in. `Fixed` looks the name up in the time zone database. Errors: `UnknownZone`, `OutOfRange`.

- A local time that happens twice (clocks go back) resolves to the first occurrence.
- A local time that never happens (clocks go forward) uses the offset from before the gap, so 02:30 in a 02:00 to 03:00 gap fires at 03:30.
- Resolving slots in order does not always give instants in order, and two slots can resolve to the same instant.

## Tests

`proptest` property tests plus pinned examples. The recurrence tests compare `next_slot` with a brute-force day-by-day enumeration written a different way. The DST tests use a POSIX rule string for US Eastern time; one test looks up `America/New_York` in the machine's time zone database.

## Dependencies

`jiff` 1.x (dates, times, zones), `uuid` (v7), `rusqlite` (bundled, unused so far), `uniffi`. `proptest` for tests.
