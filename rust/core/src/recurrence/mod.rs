//! Recurrence logic. Works in civil (wall-clock) time only: zones, and
//! therefore DST, are not involved. Turning a [`Slot`] into an instant is a
//! separate step.
//!
//! Semantics:
//! - Slots on a day are the times of day of [`Recurrence::at`], on each day the
//!   [`Rule`] selects, from [`Recurrence::starts_on`] on.
//! - `Daily`, `Monthly` and `Yearly` count `every` days, months or years from
//!   the day, month or year of `starts_on`. `Weekly` counts weeks starting on
//!   Monday, from the week containing `starts_on`.
//! - A monthly day or a yearly anchor day past the end of a short month
//!   (31 in April, February 29 in a common year) clamps to the last day of that
//!   month.
//! - [`End::Until`] is inclusive. [`End::Count`] counts individual slots.

use std::cmp::Ordering;

use jiff::{SignedDuration, civil};

use crate::model::{End, MonthlyOn, Nth, Recurrence, Rule, Slot, TimesOfDay, Watermark};

#[cfg(test)]
pub(crate) mod tests;

/// The first slot strictly after the watermark, or `None` once the recurrence
/// has ended or the calendar runs out.
///
/// `after.through` of `None` means nothing has been handled yet. `after.handled`
/// is the number of slots up to and including `through`, which is how
/// [`End::Count`] is applied without enumerating from the start.
pub fn next_slot(recurrence: &Recurrence, after: Watermark) -> Option<Slot> {
    if let Some(End::Count(limit)) = &recurrence.end
        && after.handled >= limit.get()
    {
        return None;
    }

    let mut day = match after.through {
        Some(Slot(through)) => through.date().max(recurrence.starts_on),
        None => recurrence.starts_on,
    };

    loop {
        if let Some(End::Until(until)) = &recurrence.end
            && day > until.date()
        {
            return None;
        }

        if day_matches(&recurrence.rule, recurrence.starts_on, day) {
            let bound = after
                .through
                .filter(|Slot(through)| through.date() == day)
                .map(|Slot(through)| through.time());
            if let Some(time) = first_time_after(&recurrence.at, bound) {
                let candidate = day.to_datetime(time);
                if let Some(End::Until(until)) = &recurrence.end
                    && candidate > *until
                {
                    return None;
                }
                return Some(Slot(candidate));
            }
        }

        day = day.tomorrow().ok()?;
    }
}

fn day_matches(rule: &Rule, starts_on: civil::Date, day: civil::Date) -> bool {
    if day < starts_on {
        return false;
    }
    match rule {
        Rule::Daily { every } => days_since(starts_on, day) % i64::from(every.get()) == 0,
        Rule::Weekly { every, on } => {
            let weeks = (days_since(starts_on, day)
                + i64::from(starts_on.weekday().to_monday_zero_offset()))
                / 7;
            weeks % i64::from(every.get()) == 0 && on.contains(day.weekday())
        }
        Rule::Monthly { every, on } => {
            months_since(starts_on, day) % i64::from(every.get()) == 0
                && match on {
                    MonthlyOn::Day(wanted) => {
                        let wanted = i8::try_from(wanted.get()).unwrap_or(i8::MAX);
                        day.day() == wanted.min(day.days_in_month())
                    }
                    MonthlyOn::NthWeekday { nth, weekday } => {
                        day.nth_weekday_of_month(nth_offset(*nth), *weekday).ok() == Some(day)
                    }
                }
        }
        Rule::Yearly { every } => {
            i64::from(day.year() - starts_on.year()) % i64::from(every.get()) == 0
                && day.month() == starts_on.month()
                && day.day() == starts_on.day().min(day.days_in_month())
        }
    }
}

fn nth_offset(nth: Nth) -> i8 {
    match nth {
        Nth::First => 1,
        Nth::Second => 2,
        Nth::Third => 3,
        Nth::Fourth => 4,
        Nth::Last => -1,
    }
}

fn days_since(earlier: civil::Date, later: civil::Date) -> i64 {
    earlier.duration_until(later).as_secs() / 86_400
}

fn months_since(earlier: civil::Date, later: civil::Date) -> i64 {
    (i64::from(later.year()) - i64::from(earlier.year())) * 12 + i64::from(later.month())
        - i64::from(earlier.month())
}

/// The first time of day in `at` that is strictly after `bound`, or the first
/// time of day when there is no bound.
fn first_time_after(at: &TimesOfDay, bound: Option<civil::Time>) -> Option<civil::Time> {
    match at {
        TimesOfDay::At(times) => times
            .as_slice()
            .iter()
            .copied()
            .find(|time| bound.is_none_or(|bound| *time > bound)),
        TimesOfDay::Every(window) => {
            let start = nanos_of_day(window.start());
            let end = nanos_of_day(window.end());
            let every = window.every().get().as_nanos();
            let next = match bound.map(nanos_of_day) {
                None => start,
                Some(bound) => match bound.cmp(&start) {
                    Ordering::Less => start,
                    _ => start + ((bound - start) / every + 1) * every,
                },
            };
            (next <= end).then(|| time_of_day(next))
        }
    }
}

fn nanos_of_day(time: civil::Time) -> i128 {
    time.duration_since(civil::Time::midnight()).as_nanos()
}

fn time_of_day(nanos: i128) -> civil::Time {
    civil::Time::midnight() + SignedDuration::from_nanos_i128(nanos)
}
