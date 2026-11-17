//! Property tests for the validated value types.

use jiff::{SignedDuration, civil};
use proptest::prelude::*;

use super::{ClockTimes, DayOfMonth, InvalidValue, PositiveDuration, Weekdays, Window};

/// Any time of day, to nanosecond precision, so that sub-second differences are
/// exercised.
fn any_time() -> impl Strategy<Value = civil::Time> {
    (0..86_400_000_000_000i64)
        .prop_map(|nanos| civil::Time::midnight() + SignedDuration::from_nanos(nanos))
}

/// Up to a dozen times, drawn partly from a tiny pool so duplicates are common.
fn clock_time_input() -> impl Strategy<Value = Vec<civil::Time>> {
    let coarse = (0..3i8, 0..2i8).prop_map(|(hour, minute)| civil::time(hour, minute, 0, 0));
    prop::collection::vec(prop_oneof![coarse, any_time()], 0..12)
}

fn weekday_from_offset(offset: i8) -> civil::Weekday {
    civil::Weekday::from_monday_zero_offset(offset).expect("offset is 0 to 6")
}

fn weekday_input() -> impl Strategy<Value = Vec<civil::Weekday>> {
    prop::collection::vec((0..7i8).prop_map(weekday_from_offset), 0..14)
}

proptest! {
    #[test]
    fn clock_times_are_sorted_and_unique(times in clock_time_input()) {
        match ClockTimes::new(times.clone()) {
            Err(error) => {
                prop_assert!(times.is_empty());
                prop_assert_eq!(error, InvalidValue::NoClockTimes);
            }
            Ok(held) => {
                prop_assert!(held.as_slice().windows(2).all(|pair| pair[0] < pair[1]));
            }
        }
    }

    #[test]
    fn clock_times_hold_exactly_the_given_times(times in clock_time_input()) {
        if let Ok(held) = ClockTimes::new(times.clone()) {
            for time in &times {
                prop_assert!(held.as_slice().contains(time));
            }
            for time in held.as_slice() {
                prop_assert!(times.contains(time));
            }
        }
    }

    #[test]
    fn clock_times_ignore_order_and_repeats(times in clock_time_input()) {
        let mut reordered: Vec<civil::Time> = times.iter().rev().copied().collect();
        reordered.extend(times.iter().copied());
        prop_assert_eq!(ClockTimes::new(times), ClockTimes::new(reordered));
    }

    #[test]
    fn weekdays_hold_exactly_the_given_days(days in weekday_input()) {
        match Weekdays::new(days.clone()) {
            Err(error) => {
                prop_assert!(days.is_empty());
                prop_assert_eq!(error, InvalidValue::NoWeekdays);
            }
            Ok(set) => {
                let listed: Vec<civil::Weekday> = set.iter().collect();
                let offsets: Vec<i8> = listed.iter().map(|day| day.to_monday_zero_offset()).collect();
                prop_assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
                for offset in 0..7i8 {
                    let day = weekday_from_offset(offset);
                    prop_assert_eq!(set.contains(day), days.contains(&day));
                    prop_assert_eq!(listed.contains(&day), days.contains(&day));
                }
            }
        }
    }

    #[test]
    fn weekdays_ignore_order(days in weekday_input()) {
        let reversed: Vec<civil::Weekday> = days.iter().rev().copied().collect();
        prop_assert_eq!(Weekdays::new(days), Weekdays::new(reversed));
    }

    #[test]
    fn day_of_month_accepts_exactly_1_to_31(day in any::<u8>()) {
        match DayOfMonth::new(day) {
            Ok(held) => {
                prop_assert!((1..=31).contains(&day));
                prop_assert_eq!(held.get(), day);
            }
            Err(error) => {
                prop_assert!(!(1..=31).contains(&day));
                prop_assert_eq!(error, InvalidValue::DayOfMonthOutOfRange(day));
            }
        }
    }

    #[test]
    fn positive_duration_accepts_exactly_positive(nanos in any::<i64>()) {
        let duration = SignedDuration::from_nanos(nanos);
        match PositiveDuration::new(duration) {
            Ok(held) => {
                prop_assert!(nanos > 0);
                prop_assert_eq!(held.get(), duration);
            }
            Err(error) => {
                prop_assert!(nanos <= 0);
                prop_assert_eq!(error, InvalidValue::NonPositiveDuration);
            }
        }
    }

    #[test]
    fn window_accepts_exactly_start_not_after_end(
        every_nanos in 1..i64::MAX,
        start in any_time(),
        end in any_time(),
    ) {
        let every = PositiveDuration::new(SignedDuration::from_nanos(every_nanos))
            .expect("every_nanos is positive");
        match Window::new(every, start, end) {
            Ok(window) => {
                prop_assert!(start <= end);
                prop_assert_eq!(window.every(), every);
                prop_assert_eq!(window.start(), start);
                prop_assert_eq!(window.end(), end);
            }
            Err(error) => {
                prop_assert!(start > end);
                prop_assert_eq!(error, InvalidValue::WindowReversed);
            }
        }
    }

    #[test]
    fn window_allows_a_single_point(every_nanos in 1..i64::MAX, at in any_time()) {
        let every = PositiveDuration::new(SignedDuration::from_nanos(every_nanos))
            .expect("every_nanos is positive");
        prop_assert!(Window::new(every, at, at).is_ok());
    }
}
