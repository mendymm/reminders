# Model: known gaps and deferred work

Notes on `reminder-core`'s domain model (`core/src/model/`). Not a spec; a list of what the model does not do yet.

## Not representable

### Repeat after completion

"Two weeks after I last did it." Every `Rule` is anchored to the calendar through `Recurrence::starts_on`. A rule that anchors on the last completion instead would need its own state in `Progress` (resolutions are archival and never read by logic), and a way to express the interval in the model.

### Not modelled at all

- Date-only and all-day times. `When` always has a time of day, and `Recurrence` always has `TimesOfDay`.
- Quiet hours.
- Priorities.
- Tags and categories.
- Checklists and subtasks.
- Location triggers.
- Relative times, such as "30 minutes before X".

## Deferred

- A silent alert style. Unscheduled plans carry no alert, but a dated reminder always alerts. Adding a variant to `AlertStyle` later is additive.
- `Nth` has no fifth weekday, because not every month has one. `Last` covers the use case.
- `Slot` wraps a `civil::DateTime` without normalising precision. Watermarks compare with `<` and `>`, so this matters little, but decide whether slots are whole minutes.

## Progress and resolutions

Resolutions are an archive. Scheduling and completion logic read only `Reminder` and `Progress`. A command that resolves a reminder updates `Progress` and appends a `Resolution` together.

Open points:

- **Merging `Progress` when syncing.** The watermark normally only moves forward, which would merge as a maximum. Undo moves it back, so a maximum is unsound. Use the revision (last write wins) unless a better rule turns up.
- **Undo is one level deep.** `ProgressState::Cadence::previous` holds the watermark before the last resolution. A second undo in a row has nothing to restore.
- **A watermark cannot leave a gap.** Completing Wednesday's slot closes Tuesday's as well. `Watermark::handled` counts slots passed over this way, which needs the recurrence logic to enumerate them.
- **Editing the plan resets or migrates `Progress`.** Changing a reminder between `Once`, a cadence and a tracker changes which `ProgressState` it needs.

## Invariants the types do not enforce

Single-field shapes are enforced by the types (see `model/values.rs`). These span several fields or entities, or depend on the environment, so they need a check elsewhere:

- `Progress::state` matching the reminder's `Plan`, and a tracker having no `Progress`.
- `Zone::Fixed` names are checked when a time is resolved, because validity depends on the device's time zone database.
- `End::Until` earlier than `Recurrence::starts_on`. It never fires.
- `Meta` timestamps out of order, such as `updated_at` before `created_at`.
- What `MonthlyOn::Day` 29 to 31 does in shorter months. Belongs in recurrence logic.
- An empty or blank `Reminder::title`.

The validated types hide their fields, so anything built from stored or network data must go through their `new` constructors. Do not derive `Deserialize` on them blindly.
