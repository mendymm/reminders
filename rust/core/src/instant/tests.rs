//! Tests for [`instant_of`]. The DST cases use a POSIX rule string for US Eastern
//! time, so they do not depend on the machine's time zone database. In 2026 the
//! clocks go forward on March 8 at 02:00 and back on November 1 at 02:00.

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan, civil};
use proptest::prelude::*;

use super::{ZoneError, instant_of};
use crate::model::{Slot, Zone};

fn eastern() -> TimeZone {
    TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").expect("a valid POSIX rule")
}

fn at(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> Slot {
    Slot(civil::date(year, month, day).at(hour, minute, 0, 0))
}

fn instant(text: &str) -> Timestamp {
    text.parse().expect("a valid timestamp")
}

fn floating_in_eastern(slot: Slot) -> Timestamp {
    instant_of(slot, &Zone::Floating, &eastern()).expect("a representable instant")
}

#[test]
fn an_ordinary_time_uses_the_offset_in_force() {
    assert_eq!(
        floating_in_eastern(at(2026, 1, 15, 9, 0)),
        instant("2026-01-15T14:00:00Z")
    );
    assert_eq!(
        floating_in_eastern(at(2026, 6, 15, 9, 0)),
        instant("2026-06-15T13:00:00Z")
    );
}

#[test]
fn a_time_in_the_spring_gap_uses_the_offset_from_before_the_gap() {
    // 02:30 does not exist on 2026-03-08. EST (-05:00) applied before the gap,
    // so this is 07:30Z, which is 03:30 on the clocks after the jump.
    assert_eq!(
        floating_in_eastern(at(2026, 3, 8, 2, 30)),
        instant("2026-03-08T07:30:00Z")
    );
}

#[test]
fn slots_in_the_gap_can_fire_after_a_later_slot() {
    // 03:00 EDT is 07:00Z, so the nominally earlier 02:30 slot fires after it.
    let in_gap = floating_in_eastern(at(2026, 3, 8, 2, 30));
    let after_gap = floating_in_eastern(at(2026, 3, 8, 3, 0));
    assert_eq!(after_gap, instant("2026-03-08T07:00:00Z"));
    assert!(in_gap > after_gap);
}

#[test]
fn a_time_in_the_autumn_fold_uses_the_first_occurrence() {
    // 01:30 happens twice on 2026-11-01. The first is EDT (-04:00), 05:30Z. The
    // second would be EST, 06:30Z.
    assert_eq!(
        floating_in_eastern(at(2026, 11, 1, 1, 30)),
        instant("2026-11-01T05:30:00Z")
    );
}

#[test]
fn a_floating_slot_follows_the_device_zone() {
    let slot = at(2026, 6, 15, 9, 0);
    let plus_two = TimeZone::fixed(jiff::tz::offset(2));
    let minus_five = TimeZone::fixed(jiff::tz::offset(-5));

    assert_eq!(
        instant_of(slot, &Zone::Floating, &plus_two),
        Ok(instant("2026-06-15T07:00:00Z"))
    );
    assert_eq!(
        instant_of(slot, &Zone::Floating, &minus_five),
        Ok(instant("2026-06-15T14:00:00Z"))
    );
}

#[test]
fn a_fixed_slot_ignores_the_device_zone() {
    // Looks the name up in the machine's time zone database.
    let zone = Zone::Fixed("America/New_York".to_owned());
    let slot = at(2026, 6, 15, 9, 0);
    let device = TimeZone::fixed(jiff::tz::offset(9));

    assert_eq!(
        instant_of(slot, &zone, &device),
        Ok(instant("2026-06-15T13:00:00Z"))
    );
}

#[test]
fn an_unknown_zone_name_is_an_error() {
    let zone = Zone::Fixed("Not/AZone".to_owned());
    assert_eq!(
        instant_of(at(2026, 6, 15, 9, 0), &zone, &eastern()),
        Err(ZoneError::UnknownZone("Not/AZone".to_owned()))
    );
}

#[test]
fn the_edge_of_the_calendar_is_out_of_range_not_a_panic() {
    let device = TimeZone::fixed(jiff::tz::offset(0));
    assert_eq!(
        instant_of(at(9999, 12, 31, 23, 59), &Zone::Floating, &device),
        Err(ZoneError::OutOfRange)
    );
}

fn any_slot_in_2026() -> impl Strategy<Value = Slot> {
    (0..365i32, 0..1440i32).prop_map(|(day, minute)| {
        let date = civil::date(2026, 1, 1) + day.days();
        Slot(date.at((minute / 60) as i8, (minute % 60) as i8, 0, 0))
    })
}

proptest! {
    #[test]
    fn the_instant_reads_back_as_the_slot_except_inside_the_spring_gap(slot in any_slot_in_2026()) {
        let tz = eastern();
        let instant = instant_of(slot, &Zone::Floating, &tz).expect("a representable instant");
        let back = tz.to_datetime(instant);

        let in_gap = slot.0.date() == civil::date(2026, 3, 8) && slot.0.hour() == 2;
        if in_gap {
            prop_assert_eq!(back, slot.0 + 1.hour());
        } else {
            prop_assert_eq!(back, slot.0);
        }
    }

    #[test]
    fn a_repeated_time_in_the_autumn_fold_resolves_to_the_first_occurrence(minute in 0..60i8) {
        let tz = eastern();
        let slot = at(2026, 11, 1, 1, minute);
        let instant = instant_of(slot, &Zone::Floating, &tz).expect("a representable instant");

        // EDT, not EST.
        prop_assert_eq!(tz.to_offset(instant).seconds(), -4 * 3600);
    }

    #[test]
    fn a_floating_slot_is_its_local_time_shifted_by_the_device_offset(
        slot in any_slot_in_2026(),
        hours in -12..=14i8,
    ) {
        let device = TimeZone::fixed(jiff::tz::offset(hours));
        let as_if_utc = TimeZone::UTC.to_timestamp(slot.0).expect("a representable instant");

        prop_assert_eq!(
            instant_of(slot, &Zone::Floating, &device),
            Ok(as_if_utc - SignedDuration::from_hours(i64::from(hours)))
        );
    }
}
