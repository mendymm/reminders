//! The types Kotlin sees, and their conversions to and from the model.
//!
//! They mirror the model one to one, built from what UniFFI can carry: instants
//! are Unix milliseconds, ids are UUID text, local dates and times are ISO 8601
//! text, durations are `std::time::Duration`, counts are plain integers. Turning
//! one into a model value is fallible and goes through the model's constructors,
//! so bad input comes back as [`ReminderError::InvalidInput`] and never builds a
//! value the model would refuse.
//!
//! Naming differs from the model in two places, to keep Kotlin clear of its own
//! names: [`CallContext`] (Android has a `Context`) and [`Due::at`] (`when` is a
//! Kotlin keyword).

use std::fmt;
use std::num::NonZeroU32;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp, civil};
use uuid::Uuid;

use super::error::ReminderError;
use crate::commands as cmd;
use crate::instant::ZoneError;
use crate::model as domain;

// --- helpers --------------------------------------------------------------

pub(super) fn invalid(error: impl fmt::Display) -> ReminderError {
    ReminderError::InvalidInput {
        reason: error.to_string(),
    }
}

pub(super) fn timestamp_from_ms(milliseconds: i64) -> Result<Timestamp, ReminderError> {
    Timestamp::from_millisecond(milliseconds).map_err(invalid)
}

pub(super) fn uuid_from_text(text: &str) -> Result<Uuid, ReminderError> {
    Uuid::parse_str(text).map_err(invalid)
}

fn datetime_from_text(text: &str) -> Result<civil::DateTime, ReminderError> {
    text.parse().map_err(invalid)
}

fn date_from_text(text: &str) -> Result<civil::Date, ReminderError> {
    text.parse().map_err(invalid)
}

fn time_from_text(text: &str) -> Result<civil::Time, ReminderError> {
    text.parse().map_err(invalid)
}

fn slot_from_text(text: &str) -> Result<domain::Slot, ReminderError> {
    datetime_from_text(text).map(domain::Slot)
}

fn non_zero(value: u32) -> Result<NonZeroU32, ReminderError> {
    NonZeroU32::new(value).ok_or_else(|| invalid("a count must not be zero"))
}

/// A positive model duration as a `Duration`. Positive durations always fit.
fn duration_to_ffi(duration: SignedDuration) -> Duration {
    Duration::new(
        u64::try_from(duration.as_secs()).unwrap_or(0),
        u32::try_from(duration.subsec_nanos()).unwrap_or(0),
    )
}

fn duration_from_ffi(duration: Duration) -> Result<SignedDuration, ReminderError> {
    let seconds = i64::try_from(duration.as_secs()).map_err(invalid)?;
    let nanos = i32::try_from(duration.subsec_nanos()).map_err(invalid)?;
    Ok(SignedDuration::new(seconds, nanos))
}

fn positive_duration_from_ffi(
    duration: Duration,
) -> Result<domain::PositiveDuration, ReminderError> {
    domain::PositiveDuration::new(duration_from_ffi(duration)?).map_err(invalid)
}

// --- the call context -----------------------------------------------------

/// What every call needs to know about the world: the time now and the zone the
/// device is in.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CallContext {
    pub now_ms: i64,
    /// An IANA zone name, such as `Europe/Berlin`.
    pub device_zone: String,
}

impl CallContext {
    pub(super) fn into_domain(self) -> Result<cmd::Context, ReminderError> {
        Ok(cmd::Context {
            now: timestamp_from_ms(self.now_ms)?,
            device_zone: jiff::tz::TimeZone::get(&self.device_zone)
                .map_err(|_| invalid(format!("unknown time zone {}", self.device_zone)))?,
        })
    }
}

// --- plans ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl From<civil::Weekday> for Weekday {
    fn from(day: civil::Weekday) -> Self {
        match day {
            civil::Weekday::Monday => Self::Monday,
            civil::Weekday::Tuesday => Self::Tuesday,
            civil::Weekday::Wednesday => Self::Wednesday,
            civil::Weekday::Thursday => Self::Thursday,
            civil::Weekday::Friday => Self::Friday,
            civil::Weekday::Saturday => Self::Saturday,
            civil::Weekday::Sunday => Self::Sunday,
        }
    }
}

impl From<Weekday> for civil::Weekday {
    fn from(day: Weekday) -> Self {
        match day {
            Weekday::Monday => Self::Monday,
            Weekday::Tuesday => Self::Tuesday,
            Weekday::Wednesday => Self::Wednesday,
            Weekday::Thursday => Self::Thursday,
            Weekday::Friday => Self::Friday,
            Weekday::Saturday => Self::Saturday,
            Weekday::Sunday => Self::Sunday,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Nth {
    First,
    Second,
    Third,
    Fourth,
    Last,
}

impl From<domain::Nth> for Nth {
    fn from(nth: domain::Nth) -> Self {
        match nth {
            domain::Nth::First => Self::First,
            domain::Nth::Second => Self::Second,
            domain::Nth::Third => Self::Third,
            domain::Nth::Fourth => Self::Fourth,
            domain::Nth::Last => Self::Last,
        }
    }
}

impl From<Nth> for domain::Nth {
    fn from(nth: Nth) -> Self {
        match nth {
            Nth::First => Self::First,
            Nth::Second => Self::Second,
            Nth::Third => Self::Third,
            Nth::Fourth => Self::Fourth,
            Nth::Last => Self::Last,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Zone {
    /// Follows the device's zone.
    Floating,
    /// An IANA zone name.
    Fixed { name: String },
}

impl From<domain::Zone> for Zone {
    fn from(zone: domain::Zone) -> Self {
        match zone {
            domain::Zone::Floating => Self::Floating,
            domain::Zone::Fixed(name) => Self::Fixed { name },
        }
    }
}

impl From<Zone> for domain::Zone {
    fn from(zone: Zone) -> Self {
        match zone {
            Zone::Floating => Self::Floating,
            Zone::Fixed { name } => Self::Fixed(name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct When {
    /// An ISO 8601 date and time, such as `2026-11-20T09:00:00`.
    pub local: String,
    pub zone: Zone,
}

impl From<domain::When> for When {
    fn from(when: domain::When) -> Self {
        Self {
            local: when.local.to_string(),
            zone: when.zone.into(),
        }
    }
}

impl TryFrom<When> for domain::When {
    type Error = ReminderError;

    fn try_from(when: When) -> Result<Self, ReminderError> {
        Ok(Self {
            local: datetime_from_text(&when.local)?,
            zone: when.zone.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AlertStyle {
    Notification { sticky: bool },
    Alarm,
}

impl From<domain::AlertStyle> for AlertStyle {
    fn from(style: domain::AlertStyle) -> Self {
        match style {
            domain::AlertStyle::Notification { sticky } => Self::Notification { sticky },
            domain::AlertStyle::Alarm => Self::Alarm,
        }
    }
}

impl From<AlertStyle> for domain::AlertStyle {
    fn from(style: AlertStyle) -> Self {
        match style {
            AlertStyle::Notification { sticky } => Self::Notification { sticky },
            AlertStyle::Alarm => Self::Alarm,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Nudge {
    pub every: Duration,
    pub limit: Option<u32>,
}

impl From<domain::Nudge> for Nudge {
    fn from(nudge: domain::Nudge) -> Self {
        Self {
            every: duration_to_ffi(nudge.every.get()),
            limit: nudge.limit.map(NonZeroU32::get),
        }
    }
}

impl TryFrom<Nudge> for domain::Nudge {
    type Error = ReminderError;

    fn try_from(nudge: Nudge) -> Result<Self, ReminderError> {
        Ok(Self {
            every: positive_duration_from_ffi(nudge.every)?,
            limit: nudge.limit.map(non_zero).transpose()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Alert {
    pub style: AlertStyle,
    pub nudge: Option<Nudge>,
}

impl From<domain::Alert> for Alert {
    fn from(alert: domain::Alert) -> Self {
        Self {
            style: alert.style.into(),
            nudge: alert.nudge.map(Nudge::from),
        }
    }
}

impl TryFrom<Alert> for domain::Alert {
    type Error = ReminderError;

    fn try_from(alert: Alert) -> Result<Self, ReminderError> {
        Ok(Self {
            style: alert.style.into(),
            nudge: alert.nudge.map(domain::Nudge::try_from).transpose()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MonthlyOn {
    Day { day: u8 },
    NthWeekday { nth: Nth, weekday: Weekday },
}

impl From<domain::MonthlyOn> for MonthlyOn {
    fn from(on: domain::MonthlyOn) -> Self {
        match on {
            domain::MonthlyOn::Day(day) => Self::Day { day: day.get() },
            domain::MonthlyOn::NthWeekday { nth, weekday } => Self::NthWeekday {
                nth: nth.into(),
                weekday: weekday.into(),
            },
        }
    }
}

impl TryFrom<MonthlyOn> for domain::MonthlyOn {
    type Error = ReminderError;

    fn try_from(on: MonthlyOn) -> Result<Self, ReminderError> {
        Ok(match on {
            MonthlyOn::Day { day } => {
                Self::Day(domain::DayOfMonth::new(day).map_err(invalid)?)
            }
            MonthlyOn::NthWeekday { nth, weekday } => Self::NthWeekday {
                nth: nth.into(),
                weekday: weekday.into(),
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Rule {
    Daily { every: u32 },
    Weekly { every: u32, on: Vec<Weekday> },
    Monthly { every: u32, on: MonthlyOn },
    Yearly { every: u32 },
}

impl From<domain::Rule> for Rule {
    fn from(rule: domain::Rule) -> Self {
        match rule {
            domain::Rule::Daily { every } => Self::Daily { every: every.get() },
            domain::Rule::Weekly { every, on } => Self::Weekly {
                every: every.get(),
                on: on.iter().map(Weekday::from).collect(),
            },
            domain::Rule::Monthly { every, on } => Self::Monthly {
                every: every.get(),
                on: on.into(),
            },
            domain::Rule::Yearly { every } => Self::Yearly { every: every.get() },
        }
    }
}

impl TryFrom<Rule> for domain::Rule {
    type Error = ReminderError;

    fn try_from(rule: Rule) -> Result<Self, ReminderError> {
        Ok(match rule {
            Rule::Daily { every } => Self::Daily {
                every: non_zero(every)?,
            },
            Rule::Weekly { every, on } => Self::Weekly {
                every: non_zero(every)?,
                on: domain::Weekdays::new(on.into_iter().map(civil::Weekday::from))
                    .map_err(invalid)?,
            },
            Rule::Monthly { every, on } => Self::Monthly {
                every: non_zero(every)?,
                on: on.try_into()?,
            },
            Rule::Yearly { every } => Self::Yearly {
                every: non_zero(every)?,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum TimesOfDay {
    /// Fixed times, each an ISO 8601 time such as `09:00:00`.
    At { times: Vec<String> },
    /// Repeats within a window of the day.
    Every {
        every: Duration,
        start: String,
        end: String,
    },
}

impl From<domain::TimesOfDay> for TimesOfDay {
    fn from(at: domain::TimesOfDay) -> Self {
        match at {
            domain::TimesOfDay::At(times) => Self::At {
                times: times.as_slice().iter().map(ToString::to_string).collect(),
            },
            domain::TimesOfDay::Every(window) => Self::Every {
                every: duration_to_ffi(window.every().get()),
                start: window.start().to_string(),
                end: window.end().to_string(),
            },
        }
    }
}

impl TryFrom<TimesOfDay> for domain::TimesOfDay {
    type Error = ReminderError;

    fn try_from(at: TimesOfDay) -> Result<Self, ReminderError> {
        Ok(match at {
            TimesOfDay::At { times } => {
                let parsed = times
                    .iter()
                    .map(|time| time_from_text(time))
                    .collect::<Result<Vec<_>, _>>()?;
                Self::At(domain::ClockTimes::new(parsed).map_err(invalid)?)
            }
            TimesOfDay::Every { every, start, end } => Self::Every(
                domain::Window::new(
                    positive_duration_from_ffi(every)?,
                    time_from_text(&start)?,
                    time_from_text(&end)?,
                )
                .map_err(invalid)?,
            ),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum End {
    /// No slot after this ISO 8601 date and time.
    Until { local: String },
    /// Stop after this many slots.
    Count { count: u32 },
}

impl From<domain::End> for End {
    fn from(end: domain::End) -> Self {
        match end {
            domain::End::Until(local) => Self::Until {
                local: local.to_string(),
            },
            domain::End::Count(count) => Self::Count { count: count.get() },
        }
    }
}

impl TryFrom<End> for domain::End {
    type Error = ReminderError;

    fn try_from(end: End) -> Result<Self, ReminderError> {
        Ok(match end {
            End::Until { local } => Self::Until(datetime_from_text(&local)?),
            End::Count { count } => Self::Count(non_zero(count)?),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Recurrence {
    /// An ISO 8601 date, such as `2026-11-16`.
    pub starts_on: String,
    pub zone: Zone,
    pub rule: Rule,
    pub at: TimesOfDay,
    pub end: Option<End>,
}

impl From<domain::Recurrence> for Recurrence {
    fn from(recurrence: domain::Recurrence) -> Self {
        Self {
            starts_on: recurrence.starts_on.to_string(),
            zone: recurrence.zone.into(),
            rule: recurrence.rule.into(),
            at: recurrence.at.into(),
            end: recurrence.end.map(End::from),
        }
    }
}

impl TryFrom<Recurrence> for domain::Recurrence {
    type Error = ReminderError;

    fn try_from(recurrence: Recurrence) -> Result<Self, ReminderError> {
        Ok(Self {
            starts_on: date_from_text(&recurrence.starts_on)?,
            zone: recurrence.zone.into(),
            rule: recurrence.rule.try_into()?,
            at: recurrence.at.try_into()?,
            end: recurrence.end.map(domain::End::try_from).transpose()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Due {
    /// The model calls this `when`, a Kotlin keyword.
    pub at: When,
    pub alert: Alert,
}

impl From<domain::Due> for Due {
    fn from(due: domain::Due) -> Self {
        Self {
            at: due.when.into(),
            alert: due.alert.into(),
        }
    }
}

impl TryFrom<Due> for domain::Due {
    type Error = ReminderError;

    fn try_from(due: Due) -> Result<Self, ReminderError> {
        Ok(Self {
            when: due.at.try_into()?,
            alert: due.alert.try_into()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Cadence {
    pub recurrence: Recurrence,
    pub alert: Alert,
}

impl From<domain::Cadence> for Cadence {
    fn from(cadence: domain::Cadence) -> Self {
        Self {
            recurrence: cadence.recurrence.into(),
            alert: cadence.alert.into(),
        }
    }
}

impl TryFrom<Cadence> for domain::Cadence {
    type Error = ReminderError;

    fn try_from(cadence: Cadence) -> Result<Self, ReminderError> {
        Ok(Self {
            recurrence: cadence.recurrence.try_into()?,
            alert: cadence.alert.try_into()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Plan {
    /// Finished by one completion.
    Once { due: Option<Due> },
    /// Never finishes. With no cadence it is a tracker.
    Ongoing { cadence: Option<Cadence> },
}

impl From<domain::Plan> for Plan {
    fn from(plan: domain::Plan) -> Self {
        match plan {
            domain::Plan::Once { due } => Self::Once {
                due: due.map(Due::from),
            },
            domain::Plan::Ongoing { cadence } => Self::Ongoing {
                cadence: cadence.map(Cadence::from),
            },
        }
    }
}

impl TryFrom<Plan> for domain::Plan {
    type Error = ReminderError;

    fn try_from(plan: Plan) -> Result<Self, ReminderError> {
        Ok(match plan {
            Plan::Once { due } => Self::Once {
                due: due.map(domain::Due::try_from).transpose()?,
            },
            Plan::Ongoing { cadence } => Self::Ongoing {
                cadence: cadence.map(domain::Cadence::try_from).transpose()?,
            },
        })
    }
}

// --- reminders, progress, resolutions -------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Meta {
    pub revision: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
}

impl From<domain::Meta> for Meta {
    fn from(meta: domain::Meta) -> Self {
        Self {
            revision: meta.revision,
            created_at_ms: meta.created_at.as_millisecond(),
            updated_at_ms: meta.updated_at.as_millisecond(),
            deleted_at_ms: meta.deleted_at.map(Timestamp::as_millisecond),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Reminder {
    pub id: String,
    pub title: String,
    pub notes: String,
    pub plan: Plan,
    pub archived_at_ms: Option<i64>,
    pub meta: Meta,
}

impl From<domain::Reminder> for Reminder {
    fn from(reminder: domain::Reminder) -> Self {
        Self {
            id: reminder.id.0.to_string(),
            title: reminder.title.as_str().to_owned(),
            notes: reminder.notes,
            plan: reminder.plan.into(),
            archived_at_ms: reminder.archived_at.map(Timestamp::as_millisecond),
            meta: reminder.meta.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Watermark {
    /// The last handled slot, an ISO 8601 date and time.
    pub through: Option<String>,
    pub handled: u32,
}

impl From<domain::Watermark> for Watermark {
    fn from(mark: domain::Watermark) -> Self {
        Self {
            through: mark.through.map(|slot| slot.0.to_string()),
            handled: mark.handled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ProgressState {
    Once {
        done_at_ms: Option<i64>,
    },
    Cadence {
        current: Watermark,
        previous: Option<Watermark>,
    },
}

impl From<domain::ProgressState> for ProgressState {
    fn from(state: domain::ProgressState) -> Self {
        match state {
            domain::ProgressState::Once { done_at } => Self::Once {
                done_at_ms: done_at.map(Timestamp::as_millisecond),
            },
            domain::ProgressState::Cadence { current, previous } => Self::Cadence {
                current: current.into(),
                previous: previous.map(Watermark::from),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Progress {
    pub reminder_id: String,
    pub state: ProgressState,
    pub last_resolution: Option<String>,
    pub revision: u64,
    pub updated_at_ms: i64,
}

impl From<domain::Progress> for Progress {
    fn from(progress: domain::Progress) -> Self {
        Self {
            reminder_id: progress.reminder_id.0.to_string(),
            state: progress.state.into(),
            last_resolution: progress.last_resolution.map(|id| id.0.to_string()),
            revision: progress.revision,
            updated_at_ms: progress.updated_at.as_millisecond(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Outcome {
    /// Did it. `slot` is the scheduled slot it was for, an ISO 8601 date and
    /// time, or none for a completion that is not tied to one.
    Done { slot: Option<String> },
    /// Passed on a scheduled slot.
    Skipped { slot: String },
}

impl From<domain::Outcome> for Outcome {
    fn from(outcome: domain::Outcome) -> Self {
        match outcome {
            domain::Outcome::Done { slot } => Self::Done {
                slot: slot.map(|slot| slot.0.to_string()),
            },
            domain::Outcome::Skipped { slot } => Self::Skipped {
                slot: slot.0.to_string(),
            },
        }
    }
}

impl TryFrom<Outcome> for domain::Outcome {
    type Error = ReminderError;

    fn try_from(outcome: Outcome) -> Result<Self, ReminderError> {
        Ok(match outcome {
            Outcome::Done { slot } => Self::Done {
                slot: slot.as_deref().map(slot_from_text).transpose()?,
            },
            Outcome::Skipped { slot } => Self::Skipped {
                slot: slot_from_text(&slot)?,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Resolution {
    pub id: String,
    pub reminder_id: String,
    pub resolved_at_ms: i64,
    pub outcome: Outcome,
    pub note: Option<String>,
    pub deleted_at_ms: Option<i64>,
}

impl From<domain::Resolution> for Resolution {
    fn from(resolution: domain::Resolution) -> Self {
        Self {
            id: resolution.id.0.to_string(),
            reminder_id: resolution.reminder_id.0.to_string(),
            resolved_at_ms: resolution.resolved_at.as_millisecond(),
            outcome: resolution.outcome.into(),
            note: resolution.note,
            deleted_at_ms: resolution.deleted_at.map(Timestamp::as_millisecond),
        }
    }
}

// --- command inputs and results -------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReminderDraft {
    pub title: String,
    pub notes: String,
    pub plan: Plan,
}

impl TryFrom<ReminderDraft> for cmd::ReminderDraft {
    type Error = ReminderError;

    fn try_from(draft: ReminderDraft) -> Result<Self, ReminderError> {
        Ok(Self {
            title: domain::Title::new(&draft.title).map_err(invalid)?,
            notes: draft.notes,
            plan: draft.plan.try_into()?,
        })
    }
}

/// Changes to a reminder. Fields left as `None` stay as they are.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReminderEdit {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub plan: Option<Plan>,
    /// `Some(true)` archives, `Some(false)` restores.
    pub archived: Option<bool>,
}

impl TryFrom<ReminderEdit> for cmd::ReminderEdit {
    type Error = ReminderError;

    fn try_from(edit: ReminderEdit) -> Result<Self, ReminderError> {
        Ok(Self {
            title: edit
                .title
                .map(|title| domain::Title::new(&title).map_err(invalid))
                .transpose()?,
            notes: edit.notes,
            plan: edit.plan.map(domain::Plan::try_from).transpose()?,
            archived: edit.archived,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReminderState {
    pub reminder: Reminder,
    pub progress: Option<Progress>,
}

impl From<cmd::ReminderState> for ReminderState {
    fn from(state: cmd::ReminderState) -> Self {
        Self {
            reminder: state.reminder.into(),
            progress: state.progress.map(Progress::from),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Resolved {
    pub resolution: Resolution,
    pub progress: Option<Progress>,
}

impl From<cmd::Resolved> for Resolved {
    fn from(resolved: cmd::Resolved) -> Self {
        Self {
            resolution: resolved.resolution.into(),
            progress: resolved.progress.map(Progress::from),
        }
    }
}

/// A slot waiting to alert.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Upcoming {
    pub reminder_id: String,
    pub title: String,
    pub slot: String,
    /// When it fires. In the past for an overdue slot.
    pub at_ms: i64,
    pub alert: Alert,
}

impl From<cmd::Upcoming> for Upcoming {
    fn from(upcoming: cmd::Upcoming) -> Self {
        Self {
            reminder_id: upcoming.reminder_id.0.to_string(),
            title: upcoming.title.as_str().to_owned(),
            slot: upcoming.slot.0.to_string(),
            at_ms: upcoming.at.as_millisecond(),
            alert: upcoming.alert.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum UnresolvedReason {
    UnknownZone { name: String },
    ZoneOutOfRange,
    MissingProgress,
}

impl From<cmd::UnresolvedReason> for UnresolvedReason {
    fn from(reason: cmd::UnresolvedReason) -> Self {
        match reason {
            cmd::UnresolvedReason::Zone(ZoneError::UnknownZone(name)) => {
                Self::UnknownZone { name }
            }
            cmd::UnresolvedReason::Zone(ZoneError::OutOfRange) => Self::ZoneOutOfRange,
            cmd::UnresolvedReason::MissingProgress => Self::MissingProgress,
        }
    }
}

/// A reminder that could not be scheduled.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Unresolved {
    pub reminder_id: String,
    pub reason: UnresolvedReason,
}

impl From<cmd::Unresolved> for Unresolved {
    fn from(unresolved: cmd::Unresolved) -> Self {
        Self {
            reminder_id: unresolved.reminder_id.0.to_string(),
            reason: unresolved.reason.into(),
        }
    }
}

/// Upcoming slots in the order they fire, plus reminders that could not be
/// scheduled.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UpcomingList {
    pub upcoming: Vec<Upcoming>,
    pub unresolved: Vec<Unresolved>,
}

impl From<cmd::UpcomingList> for UpcomingList {
    fn from(list: cmd::UpcomingList) -> Self {
        Self {
            upcoming: list.upcoming.into_iter().map(Upcoming::from).collect(),
            unresolved: list.unresolved.into_iter().map(Unresolved::from).collect(),
        }
    }
}
