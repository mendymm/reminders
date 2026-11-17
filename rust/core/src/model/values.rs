//! Validated value types.
//!
//! Each type keeps its field private, so the only way to hold one is to have
//! gone through `new`. Anything that builds these from outside the process
//! (storage, the network) must go through `new` as well.

use std::fmt;

use jiff::{SignedDuration, civil};

/// Why a validated value could not be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidValue {
    NoWeekdays,
    DayOfMonthOutOfRange(u8),
    NoClockTimes,
    NonPositiveDuration,
    WindowReversed,
    BlankTitle,
}

impl fmt::Display for InvalidValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoWeekdays => write!(f, "at least one weekday is required"),
            Self::DayOfMonthOutOfRange(day) => {
                write!(f, "day of month must be 1 to 31, got {day}")
            }
            Self::NoClockTimes => write!(f, "at least one clock time is required"),
            Self::NonPositiveDuration => write!(f, "duration must be positive"),
            Self::WindowReversed => write!(f, "window must not start after it ends"),
            Self::BlankTitle => write!(f, "title must not be blank"),
        }
    }
}

impl std::error::Error for InvalidValue {}

/// A non-empty set of weekdays. Equality ignores the order they were given in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Weekdays(u8);

impl Weekdays {
    pub fn new(days: impl IntoIterator<Item = civil::Weekday>) -> Result<Self, InvalidValue> {
        let bits = days
            .into_iter()
            .fold(0u8, |bits, day| bits | Self::bit(day));
        if bits == 0 {
            Err(InvalidValue::NoWeekdays)
        } else {
            Ok(Self(bits))
        }
    }

    pub fn contains(self, day: civil::Weekday) -> bool {
        self.0 & Self::bit(day) != 0
    }

    /// The weekdays in the set, Monday first.
    pub fn iter(self) -> impl Iterator<Item = civil::Weekday> {
        (0..7i8)
            .filter(move |offset| self.0 & (1u8 << *offset) != 0)
            .filter_map(|offset| civil::Weekday::from_monday_zero_offset(offset).ok())
    }

    fn bit(day: civil::Weekday) -> u8 {
        1u8 << day.to_monday_zero_offset()
    }
}

/// A day of the month, 1 to 31.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DayOfMonth(u8);

impl DayOfMonth {
    pub fn new(day: u8) -> Result<Self, InvalidValue> {
        if (1..=31).contains(&day) {
            Ok(Self(day))
        } else {
            Err(InvalidValue::DayOfMonthOutOfRange(day))
        }
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

/// A duration greater than zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PositiveDuration(SignedDuration);

impl PositiveDuration {
    pub fn new(duration: SignedDuration) -> Result<Self, InvalidValue> {
        if duration.is_positive() {
            Ok(Self(duration))
        } else {
            Err(InvalidValue::NonPositiveDuration)
        }
    }

    pub fn get(self) -> SignedDuration {
        self.0
    }
}

/// A non-empty set of clock times, sorted and without duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockTimes(Vec<civil::Time>);

impl ClockTimes {
    pub fn new(times: impl IntoIterator<Item = civil::Time>) -> Result<Self, InvalidValue> {
        let mut times: Vec<civil::Time> = times.into_iter().collect();
        times.sort();
        times.dedup();
        if times.is_empty() {
            Err(InvalidValue::NoClockTimes)
        } else {
            Ok(Self(times))
        }
    }

    pub fn as_slice(&self) -> &[civil::Time] {
        &self.0
    }
}

/// A repeating window within a day: first at `start`, then every `every`, never
/// later than `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Window {
    every: PositiveDuration,
    start: civil::Time,
    end: civil::Time,
}

impl Window {
    pub fn new(
        every: PositiveDuration,
        start: civil::Time,
        end: civil::Time,
    ) -> Result<Self, InvalidValue> {
        if start > end {
            Err(InvalidValue::WindowReversed)
        } else {
            Ok(Self { every, start, end })
        }
    }

    pub fn every(self) -> PositiveDuration {
        self.every
    }

    pub fn start(self) -> civil::Time {
        self.start
    }

    pub fn end(self) -> civil::Time {
        self.end
    }
}

/// A reminder title: not blank, with surrounding whitespace trimmed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Title(String);

impl Title {
    pub fn new(text: impl AsRef<str>) -> Result<Self, InvalidValue> {
        let trimmed = text.as_ref().trim();
        if trimmed.is_empty() {
            Err(InvalidValue::BlankTitle)
        } else {
            Ok(Self(trimmed.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
