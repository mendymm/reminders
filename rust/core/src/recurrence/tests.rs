//! Tests for [`next_slot`]: pinned examples for each semantic decision, and
//! property tests that compare it with a brute-force enumeration.

use std::num::NonZeroU32;

use jiff::{SignedDuration, ToSpan, civil};
use proptest::prelude::*;

use super::next_slot;
use crate::model::{
    ClockTimes, DayOfMonth, End, MonthlyOn, Nth, PositiveDuration, Recurrence, Rule, Slot,
    TimesOfDay, Watermark, Weekdays, Window, Zone,
};

fn nz(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).expect("n is not zero")
}

fn slot(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Slot {
    Slot(civil::date(year, month, day).at(hour, minute, 0, 0))
}

fn recurrence(
    starts_on: civil::Date,
    rule: Rule,
    at: TimesOfDay,
    end: Option<End>,
) -> Recurrence {
    Recurrence {
        starts_on,
        zone: Zone::Floating,
        rule,
        at,
        end,
    }
}

fn at_times(times: &[(i8, i8)]) -> TimesOfDay {
    TimesOfDay::At(
        ClockTimes::new(times.iter().map(|&(h, m)| civil::time(h, m, 0, 0)))
            .expect("at least one time"),
    )
}

fn start() -> Watermark {
    Watermark {
        through: None,
        handled: 0,
    }
}

/// Up to `limit` slots, chaining `next_slot` from the start.
fn collect(rec: &Recurrence, limit: usize) -> Vec<Slot> {
    let mut slots = Vec::new();
    let mut mark = start();
    while slots.len() < limit {
        let Some(next) = next_slot(rec, mark) else {
            break;
        };
        slots.push(next);
        mark = Watermark {
            through: Some(next),
            handled: mark.handled + 1,
        };
    }
    slots
}

#[test]
fn monthly_day_31_clamps_to_the_last_day_of_short_months() {
    let rec = recurrence(
        civil::date(2026, 1, 31),
        Rule::Monthly {
            every: nz(1),
            on: MonthlyOn::Day(DayOfMonth::new(31).expect("1 to 31")),
        },
        at_times(&[(9, 0)]),
        None,
    );
    assert_eq!(
        collect(&rec, 5),
        vec![
            slot(2026, 1, 31, 9, 0),
            slot(2026, 2, 28, 9, 0),
            slot(2026, 3, 31, 9, 0),
            slot(2026, 4, 30, 9, 0),
            slot(2026, 5, 31, 9, 0),
        ]
    );
}

#[test]
fn yearly_february_29_clamps_to_the_28th_in_common_years() {
    let rec = recurrence(
        civil::date(2024, 2, 29),
        Rule::Yearly { every: nz(1) },
        at_times(&[(9, 0)]),
        None,
    );
    assert_eq!(
        collect(&rec, 5),
        vec![
            slot(2024, 2, 29, 9, 0),
            slot(2025, 2, 28, 9, 0),
            slot(2026, 2, 28, 9, 0),
            slot(2027, 2, 28, 9, 0),
            slot(2028, 2, 29, 9, 0),
        ]
    );
}

#[test]
fn monthly_last_friday() {
    let rec = recurrence(
        civil::date(2026, 11, 1),
        Rule::Monthly {
            every: nz(1),
            on: MonthlyOn::NthWeekday {
                nth: Nth::Last,
                weekday: civil::Weekday::Friday,
            },
        },
        at_times(&[(18, 0)]),
        None,
    );
    assert_eq!(
        collect(&rec, 2),
        vec![slot(2026, 11, 27, 18, 0), slot(2026, 12, 25, 18, 0)]
    );
}

#[test]
fn weekly_every_two_weeks_counts_weeks_from_the_monday_of_the_start_week() {
    // 2026-11-18 is a Wednesday. Its week starts Monday the 16th, which is
    // before `starts_on`, so only the Wednesday counts from week zero.
    let rec = recurrence(
        civil::date(2026, 11, 18),
        Rule::Weekly {
            every: nz(2),
            on: Weekdays::new([civil::Weekday::Monday, civil::Weekday::Wednesday])
                .expect("two days"),
        },
        at_times(&[(9, 0)]),
        None,
    );
    assert_eq!(
        collect(&rec, 4),
        vec![
            slot(2026, 11, 18, 9, 0),
            slot(2026, 11, 30, 9, 0),
            slot(2026, 12, 2, 9, 0),
            slot(2026, 12, 14, 9, 0),
        ]
    );
}

#[test]
fn monthly_day_starts_on_the_first_matching_day_after_starts_on() {
    let rec = recurrence(
        civil::date(2026, 11, 16),
        Rule::Monthly {
            every: nz(1),
            on: MonthlyOn::Day(DayOfMonth::new(1).expect("1 to 31")),
        },
        at_times(&[(9, 0)]),
        None,
    );
    assert_eq!(collect(&rec, 1), vec![slot(2026, 12, 1, 9, 0)]);
}

#[test]
fn until_is_inclusive() {
    let rec = recurrence(
        civil::date(2026, 11, 16),
        Rule::Daily { every: nz(1) },
        at_times(&[(9, 0)]),
        Some(End::Until(civil::date(2026, 11, 18).at(9, 0, 0, 0))),
    );
    assert_eq!(
        collect(&rec, 10),
        vec![
            slot(2026, 11, 16, 9, 0),
            slot(2026, 11, 17, 9, 0),
            slot(2026, 11, 18, 9, 0),
        ]
    );
}

#[test]
fn count_counts_slots_not_days() {
    let rec = recurrence(
        civil::date(2026, 11, 16),
        Rule::Daily { every: nz(1) },
        at_times(&[(9, 0), (21, 0)]),
        Some(End::Count(nz(3))),
    );
    let slots = collect(&rec, 10);
    assert_eq!(
        slots,
        vec![
            slot(2026, 11, 16, 9, 0),
            slot(2026, 11, 16, 21, 0),
            slot(2026, 11, 17, 9, 0),
        ]
    );
    let exhausted = Watermark {
        through: slots.last().copied(),
        handled: 3,
    };
    assert_eq!(next_slot(&rec, exhausted), None);
}

fn two_hourly_window() -> TimesOfDay {
    TimesOfDay::Every(
        Window::new(
            PositiveDuration::new(SignedDuration::from_hours(2)).expect("positive"),
            civil::time(8, 0, 0, 0),
            civil::time(13, 0, 0, 0),
        )
        .expect("start is not after end"),
    )
}

#[test]
fn a_window_steps_through_the_day_and_stops_at_its_end() {
    let rec = recurrence(
        civil::date(2026, 11, 16),
        Rule::Daily { every: nz(1) },
        two_hourly_window(),
        None,
    );
    assert_eq!(
        collect(&rec, 4),
        vec![
            slot(2026, 11, 16, 8, 0),
            slot(2026, 11, 16, 10, 0),
            slot(2026, 11, 16, 12, 0),
            slot(2026, 11, 17, 8, 0),
        ]
    );
}

#[test]
fn resuming_mid_window_continues_the_same_day() {
    let rec = recurrence(
        civil::date(2026, 11, 16),
        Rule::Daily { every: nz(1) },
        two_hourly_window(),
        None,
    );
    let mark = Watermark {
        through: Some(slot(2026, 11, 16, 10, 0)),
        handled: 2,
    };
    assert_eq!(next_slot(&rec, mark), Some(slot(2026, 11, 16, 12, 0)));
}

#[test]
fn a_watermark_before_starts_on_does_not_pull_slots_earlier() {
    let rec = recurrence(
        civil::date(2026, 11, 16),
        Rule::Daily { every: nz(1) },
        at_times(&[(9, 0)]),
        None,
    );
    let mark = Watermark {
        through: Some(slot(2026, 11, 1, 9, 0)),
        handled: 0,
    };
    assert_eq!(next_slot(&rec, mark), Some(slot(2026, 11, 16, 9, 0)));
}

#[test]
fn the_end_of_the_calendar_ends_the_recurrence() {
    let last_day = civil::date(9999, 12, 31);
    let rec = recurrence(
        last_day,
        Rule::Daily { every: nz(1) },
        at_times(&[(9, 0)]),
        None,
    );
    let last = slot(9999, 12, 31, 9, 0);
    assert_eq!(next_slot(&rec, start()), Some(last));
    let mark = Watermark {
        through: Some(last),
        handled: 1,
    };
    assert_eq!(next_slot(&rec, mark), None);
}

// ---------------------------------------------------------------------------
// Differential property tests against a brute-force enumeration.
// ---------------------------------------------------------------------------

const HORIZON_DAYS: i32 = 800;

/// Every slot from `starts_on` through `last_day`, found by walking the calendar
/// one day at a time and counting weeks, months and years as they roll over.
/// Deliberately a different formulation from `next_slot`.
fn oracle(rec: &Recurrence, last_day: civil::Date) -> Vec<Slot> {
    let times = oracle_times(&rec.at);
    let mut slots = Vec::new();
    let mut day = rec.starts_on;
    let mut days_walked = 0i64;
    let mut mondays_passed = 0i64;
    let mut months_passed = 0i64;
    let mut years_passed = 0i64;
    while day <= last_day {
        if day > rec.starts_on {
            if day.weekday() == civil::Weekday::Monday {
                mondays_passed += 1;
            }
            if day.day() == 1 {
                months_passed += 1;
                if day.month() == 1 {
                    years_passed += 1;
                }
            }
        }
        let selected = match &rec.rule {
            Rule::Daily { every } => days_walked % i64::from(every.get()) == 0,
            Rule::Weekly { every, on } => {
                mondays_passed % i64::from(every.get()) == 0
                    && on.iter().any(|weekday| weekday == day.weekday())
            }
            Rule::Monthly { every, on } => {
                months_passed % i64::from(every.get()) == 0
                    && match on {
                        MonthlyOn::Day(wanted) => {
                            let wanted = i8::try_from(wanted.get()).expect("at most 31");
                            clamped(day.year(), day.month(), wanted) == day
                        }
                        MonthlyOn::NthWeekday { nth, weekday } => {
                            nth_weekday(day, *nth, *weekday) == day
                        }
                    }
            }
            Rule::Yearly { every } => {
                years_passed % i64::from(every.get()) == 0
                    && clamped(day.year(), rec.starts_on.month(), rec.starts_on.day()) == day
            }
        };
        if selected {
            slots.extend(times.iter().map(|time| Slot(day.to_datetime(*time))));
        }
        days_walked += 1;
        match day.tomorrow() {
            Ok(next) => day = next,
            Err(_) => break,
        }
    }
    match &rec.end {
        Some(End::Until(until)) => slots.retain(|Slot(slot)| slot <= until),
        Some(End::Count(count)) => slots.truncate(count.get() as usize),
        None => {}
    }
    slots
}

fn oracle_times(at: &TimesOfDay) -> Vec<civil::Time> {
    match at {
        TimesOfDay::At(times) => times.as_slice().to_vec(),
        TimesOfDay::Every(window) => {
            let step = window.every().get().as_nanos();
            let end = window.end().duration_since(civil::Time::midnight()).as_nanos();
            let mut nanos = window
                .start()
                .duration_since(civil::Time::midnight())
                .as_nanos();
            let mut times = Vec::new();
            while nanos <= end {
                times.push(civil::Time::midnight() + SignedDuration::from_nanos_i128(nanos));
                nanos += step;
            }
            times
        }
    }
}

/// The date for `day` in the month, or the month's last day when it has fewer.
fn clamped(year: i16, month: i8, day: i8) -> civil::Date {
    civil::Date::new(year, month, day)
        .unwrap_or_else(|_| civil::date(year, month, 1).last_of_month())
}

/// The date of the nth weekday in the month of `day`, found by listing them.
fn nth_weekday(day: civil::Date, nth: Nth, weekday: civil::Weekday) -> civil::Date {
    let mut matching = Vec::new();
    let mut date = day.first_of_month();
    while date.month() == day.month() {
        if date.weekday() == weekday {
            matching.push(date);
        }
        match date.tomorrow() {
            Ok(next) => date = next,
            Err(_) => break,
        }
    }
    match nth {
        Nth::First => matching[0],
        Nth::Second => matching[1],
        Nth::Third => matching[2],
        Nth::Fourth => matching[3],
        Nth::Last => *matching.last().expect("a month has every weekday"),
    }
}

fn weekday_from_offset(offset: i8) -> civil::Weekday {
    civil::Weekday::from_monday_zero_offset(offset).expect("offset is 0 to 6")
}

fn time_of_day_secs(seconds: i64) -> civil::Time {
    civil::Time::midnight() + SignedDuration::from_secs(seconds)
}

fn any_start() -> impl Strategy<Value = civil::Date> {
    (0..2000i32).prop_map(|days| civil::date(2020, 1, 1) + days.days())
}

fn any_weekdays() -> impl Strategy<Value = Weekdays> {
    (1u8..128).prop_map(|bits| {
        Weekdays::new(
            (0..7i8)
                .filter(|offset| bits & (1u8 << *offset) != 0)
                .map(weekday_from_offset),
        )
        .expect("bits is not zero")
    })
}

fn any_nth() -> impl Strategy<Value = Nth> {
    prop_oneof![
        Just(Nth::First),
        Just(Nth::Second),
        Just(Nth::Third),
        Just(Nth::Fourth),
        Just(Nth::Last),
    ]
}

fn any_monthly_on() -> impl Strategy<Value = MonthlyOn> {
    prop_oneof![
        (1..=31u8).prop_map(|day| MonthlyOn::Day(DayOfMonth::new(day).expect("1 to 31"))),
        (any_nth(), (0..7i8).prop_map(weekday_from_offset))
            .prop_map(|(nth, weekday)| MonthlyOn::NthWeekday { nth, weekday }),
    ]
}

fn any_rule() -> impl Strategy<Value = Rule> {
    prop_oneof![
        (1..=10u32).prop_map(|every| Rule::Daily { every: nz(every) }),
        (1..=4u32, any_weekdays()).prop_map(|(every, on)| Rule::Weekly {
            every: nz(every),
            on,
        }),
        (1..=4u32, any_monthly_on()).prop_map(|(every, on)| Rule::Monthly {
            every: nz(every),
            on,
        }),
        (1..=2u32).prop_map(|every| Rule::Yearly { every: nz(every) }),
    ]
}

fn any_times_of_day() -> impl Strategy<Value = TimesOfDay> {
    prop_oneof![
        prop::collection::vec((0..24i8, 0..60i8), 1..4).prop_map(|times| {
            TimesOfDay::At(
                ClockTimes::new(
                    times
                        .into_iter()
                        .map(|(hour, minute)| civil::time(hour, minute, 0, 0)),
                )
                .expect("at least one time"),
            )
        }),
        // Start and end to the nanosecond, so the stepping arithmetic is
        // exercised on times that do not fall on round numbers.
        (
            0..86_400_000_000_000i64,
            0..=21_600_000_000_000i64,
            900..=7_200i64
        )
            .prop_map(|(start, length, every)| {
                let end = (start + length).min(86_399_999_999_999);
                TimesOfDay::Every(
                    Window::new(
                        PositiveDuration::new(SignedDuration::from_secs(every)).expect("positive"),
                        civil::Time::midnight() + SignedDuration::from_nanos(start),
                        civil::Time::midnight() + SignedDuration::from_nanos(end),
                    )
                    .expect("start is not after end"),
                )
            }),
    ]
}

fn any_end() -> impl Strategy<Value = Option<End>> {
    prop_oneof![
        Just(None),
        (0..2800i32, 0..86_400i64).prop_map(|(days, seconds)| {
            Some(End::Until(
                (civil::date(2020, 1, 1) + days.days()).to_datetime(time_of_day_secs(seconds)),
            ))
        }),
        (1..=30u32).prop_map(|count| Some(End::Count(nz(count)))),
    ]
}

pub(crate) fn any_recurrence() -> impl Strategy<Value = Recurrence> {
    (any_start(), any_rule(), any_times_of_day(), any_end())
        .prop_map(|(starts_on, rule, at, end)| recurrence(starts_on, rule, at, end))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn chaining_next_slot_reproduces_the_brute_force_enumeration(rec in any_recurrence()) {
        let last_day = rec.starts_on + HORIZON_DAYS.days();
        let expected = oracle(&rec, last_day);

        let mut actual = Vec::new();
        let mut mark = start();
        while let Some(next) = next_slot(&rec, mark) {
            if next.0.date() > last_day {
                break;
            }
            actual.push(next);
            mark = Watermark { through: Some(next), handled: mark.handled + 1 };
        }

        prop_assert_eq!(actual, expected);
    }

    #[test]
    fn resuming_from_a_slot_continues_the_sequence(
        rec in any_recurrence(),
        pick in any::<prop::sample::Index>(),
    ) {
        let expected = oracle(&rec, rec.starts_on + HORIZON_DAYS.days());
        prop_assume!(expected.len() >= 2);
        let index = pick.index(expected.len() - 1);
        let mark = Watermark {
            through: Some(expected[index]),
            handled: u32::try_from(index + 1).expect("a small count"),
        };

        prop_assert_eq!(next_slot(&rec, mark), Some(expected[index + 1]));
    }

    #[test]
    fn resuming_from_any_time_finds_the_next_slot_after_it(
        rec in any_recurrence(),
        offset_seconds in 0..(i64::from(HORIZON_DAYS) * 86_400),
    ) {
        let last_day = rec.starts_on + HORIZON_DAYS.days();
        let through = rec.starts_on.to_datetime(civil::Time::midnight())
            + SignedDuration::from_secs(offset_seconds);
        let all = oracle(&rec, last_day);
        let handled = all.iter().filter(|Slot(slot)| *slot <= through).count();
        let expected = all.iter().find(|Slot(slot)| *slot > through).copied();

        let actual = next_slot(
            &rec,
            Watermark {
                through: Some(Slot(through)),
                handled: u32::try_from(handled).expect("a small count"),
            },
        );

        match expected {
            Some(_) => prop_assert_eq!(actual, expected),
            // The enumeration stopped at the horizon, so a slot just beyond it is
            // fine. Anything inside the horizon would be a missed slot.
            None => prop_assert!(actual.is_none_or(|Slot(slot)| slot.date() > last_day)),
        }
    }
}
