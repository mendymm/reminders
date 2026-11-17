//! Validated value types.
//!
//! Each type keeps its field private, so the only way to hold one is to have
//! gone through `new`. Anything that builds these from outside the process
//! (storage, the network) must go through `new` as well.

use std::fmt;

use jiff::{SignedDuration, civil};
use serde::{Deserialize, Serialize};

/// Why a validated value could not be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidValue {
    NoWeekdays,
    DayOfMonthOutOfRange(u8),
    NoClockTimes,
    NonPositiveDuration,
    WindowReversed,
    BlankTitle,
    WeekdayOutOfRange(u8),
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
            Self::WeekdayOutOfRange(offset) => {
                write!(f, "weekday offset must be 0 to 6, got {offset}")
            }
        }
    }
}

impl std::error::Error for InvalidValue {}

/// A non-empty set of weekdays. Equality ignores the order they were given in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Vec<u8>", into = "Vec<u8>")]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "SignedDuration", into = "SignedDuration")]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<civil::Time>", into = "Vec<civil::Time>")]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "WindowRaw", into = "WindowRaw")]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
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

// Serde goes through the constructors, so stored or received data cannot build
// a value that `new` would refuse.

impl From<Weekdays> for Vec<u8> {
    /// Monday-based offsets, 0 to 6, Monday first.
    fn from(days: Weekdays) -> Self {
        days.iter()
            .map(|day| u8::try_from(day.to_monday_zero_offset()).unwrap_or(0))
            .collect()
    }
}

impl TryFrom<Vec<u8>> for Weekdays {
    type Error = InvalidValue;

    fn try_from(offsets: Vec<u8>) -> Result<Self, Self::Error> {
        let days = offsets
            .into_iter()
            .map(|offset| {
                i8::try_from(offset)
                    .ok()
                    .and_then(|signed| civil::Weekday::from_monday_zero_offset(signed).ok())
                    .ok_or(InvalidValue::WeekdayOutOfRange(offset))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(days)
    }
}

impl TryFrom<u8> for DayOfMonth {
    type Error = InvalidValue;

    fn try_from(day: u8) -> Result<Self, Self::Error> {
        Self::new(day)
    }
}

impl From<DayOfMonth> for u8 {
    fn from(day: DayOfMonth) -> Self {
        day.get()
    }
}

impl TryFrom<SignedDuration> for PositiveDuration {
    type Error = InvalidValue;

    fn try_from(duration: SignedDuration) -> Result<Self, Self::Error> {
        Self::new(duration)
    }
}

impl From<PositiveDuration> for SignedDuration {
    fn from(duration: PositiveDuration) -> Self {
        duration.get()
    }
}

impl TryFrom<Vec<civil::Time>> for ClockTimes {
    type Error = InvalidValue;

    fn try_from(times: Vec<civil::Time>) -> Result<Self, Self::Error> {
        Self::new(times)
    }
}

impl From<ClockTimes> for Vec<civil::Time> {
    fn from(times: ClockTimes) -> Self {
        times.0
    }
}

/// The serialized form of a [`Window`].
#[derive(Serialize, Deserialize)]
struct WindowRaw {
    every: PositiveDuration,
    start: civil::Time,
    end: civil::Time,
}

impl TryFrom<WindowRaw> for Window {
    type Error = InvalidValue;

    fn try_from(raw: WindowRaw) -> Result<Self, Self::Error> {
        Self::new(raw.every, raw.start, raw.end)
    }
}

impl From<Window> for WindowRaw {
    fn from(window: Window) -> Self {
        Self {
            every: window.every(),
            start: window.start(),
            end: window.end(),
        }
    }
}

impl TryFrom<String> for Title {
    type Error = InvalidValue;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(text)
    }
}

impl From<Title> for String {
    fn from(title: Title) -> Self {
        title.0
    }
}

/// Serde for `jiff::civil::Weekday`, which has none: a Monday-based offset, 0 to 6.
pub(super) mod weekday {
    use jiff::civil;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::InvalidValue;

    pub fn serialize<S: Serializer>(day: &civil::Weekday, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(u8::try_from(day.to_monday_zero_offset()).unwrap_or(0))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<civil::Weekday, D::Error> {
        let offset = u8::deserialize(deserializer)?;
        i8::try_from(offset)
            .ok()
            .and_then(|signed| civil::Weekday::from_monday_zero_offset(signed).ok())
            .ok_or_else(|| D::Error::custom(InvalidValue::WeekdayOutOfRange(offset)))
    }
}
