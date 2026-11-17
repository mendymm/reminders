//! Property tests for the validated value types.

use jiff::{SignedDuration, Timestamp, civil};
use proptest::prelude::*;

use super::{
    ClockTimes, DayOfMonth, InvalidValue, MonthlyOn, PositiveDuration, ReminderId, ResolutionId,
    Title, Weekdays, Window,
};

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

/// Instants from the epoch to about the year 2096, with nanoseconds.
fn any_instant() -> impl Strategy<Value = Timestamp> {
    (0..4_000_000_000i64, 0..1_000_000_000i32)
        .prop_map(|(seconds, nanos)| Timestamp::new(seconds, nanos).expect("in range"))
}

proptest! {
    #[test]
    fn ids_are_version_7_and_carry_their_instant_to_the_millisecond(now in any_instant()) {
        for id in [ReminderId::new(now).0, ResolutionId::new(now).0] {
            prop_assert_eq!(id.get_version_num(), 7);
            let (seconds, nanos) = id.get_timestamp().expect("a v7 id has a timestamp").to_unix();
            prop_assert_eq!(seconds, u64::try_from(now.as_second()).expect("after the epoch"));
            let millisecond_nanos = u32::try_from(now.subsec_nanosecond()).expect("after the epoch") / 1_000_000 * 1_000_000;
            prop_assert_eq!(nanos, millisecond_nanos);
        }
    }

    #[test]
    fn ids_sort_by_the_instant_they_were_made_at(earlier in any_instant(), later in any_instant()) {
        prop_assume!(earlier.as_millisecond() < later.as_millisecond());
        prop_assert!(ReminderId::new(earlier).0 < ReminderId::new(later).0);
        prop_assert!(ResolutionId::new(earlier).0 < ResolutionId::new(later).0);
    }
}

#[test]
fn ids_made_at_the_same_instant_still_differ() {
    let now = Timestamp::from_second(1_800_000_000).expect("in range");
    assert_ne!(ReminderId::new(now), ReminderId::new(now));
}

#[test]
fn an_instant_before_the_epoch_is_stamped_as_the_epoch() {
    let before = Timestamp::from_second(-5).expect("in range");
    let id = ReminderId::new(before).0;
    assert_eq!(id.get_version_num(), 7);
    assert_eq!(id.get_timestamp().expect("a v7 id has a timestamp").to_unix(), (0, 0));
}

proptest! {
    #[test]
    fn title_is_trimmed_and_never_blank(text in "[ \t\na-z]{0,12}") {
        match Title::new(&text) {
            Ok(title) => {
                prop_assert!(!text.trim().is_empty());
                prop_assert_eq!(title.as_str(), text.trim());
            }
            Err(error) => {
                prop_assert!(text.trim().is_empty());
                prop_assert_eq!(error, InvalidValue::BlankTitle);
            }
        }
    }
}

#[test]
fn json_the_validated_types_would_not_build_is_refused() {
    assert!(serde_json::from_str::<Weekdays>("[]").is_err());
    assert!(serde_json::from_str::<Weekdays>("[7]").is_err());
    assert!(serde_json::from_str::<DayOfMonth>("0").is_err());
    assert!(serde_json::from_str::<DayOfMonth>("32").is_err());
    assert!(serde_json::from_str::<PositiveDuration>("\"PT0S\"").is_err());
    assert!(serde_json::from_str::<PositiveDuration>("\"-PT1S\"").is_err());
    assert!(serde_json::from_str::<ClockTimes>("[]").is_err());
    assert!(
        serde_json::from_str::<Window>(
            r#"{"every":"PT1H","start":"10:00:00","end":"09:00:00"}"#
        )
        .is_err()
    );
    assert!(serde_json::from_str::<Title>("\"   \"").is_err());
    assert!(
        serde_json::from_str::<MonthlyOn>(r#"{"NthWeekday":{"nth":"Last","weekday":9}}"#).is_err()
    );
}

#[test]
fn json_the_validated_types_would_build_is_accepted() {
    assert!(serde_json::from_str::<Weekdays>("[0,4]").is_ok());
    assert!(serde_json::from_str::<DayOfMonth>("31").is_ok());
    assert!(serde_json::from_str::<PositiveDuration>("\"PT1S\"").is_ok());
    assert!(serde_json::from_str::<ClockTimes>(r#"["09:00:00"]"#).is_ok());
    assert!(
        serde_json::from_str::<Window>(
            r#"{"every":"PT1H","start":"09:00:00","end":"10:00:00"}"#
        )
        .is_ok()
    );
    assert!(serde_json::from_str::<Title>("\"Water plants\"").is_ok());
    assert!(
        serde_json::from_str::<MonthlyOn>(r#"{"NthWeekday":{"nth":"Last","weekday":4}}"#).is_ok()
    );
}

proptest! {
    #[test]
    fn weekdays_survive_json(days in weekday_input()) {
        prop_assume!(!days.is_empty());
        let set = Weekdays::new(days).expect("not empty");
        let json = serde_json::to_string(&set).expect("serialized");
        prop_assert_eq!(serde_json::from_str::<Weekdays>(&json).expect("parsed"), set);
    }

    #[test]
    fn clock_times_survive_json_with_their_nanoseconds(times in clock_time_input()) {
        prop_assume!(!times.is_empty());
        let held = ClockTimes::new(times).expect("not empty");
        let json = serde_json::to_string(&held).expect("serialized");
        prop_assert_eq!(serde_json::from_str::<ClockTimes>(&json).expect("parsed"), held);
    }

    #[test]
    fn positive_durations_survive_json(seconds in 0..1_000_000i64, nanos in 0..1_000_000_000i32) {
        prop_assume!(seconds > 0 || nanos > 0);
        let duration = PositiveDuration::new(SignedDuration::new(seconds, nanos)).expect("positive");
        let json = serde_json::to_string(&duration).expect("serialized");
        prop_assert_eq!(serde_json::from_str::<PositiveDuration>(&json).expect("parsed"), duration);
    }
}
