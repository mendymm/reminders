//! Turning a [`Slot`], a nominal local time, into the instant it fires at.
//!
//! A slot's identity stays its local time. This module only answers when that
//! local time happens on a clock in a given zone.
//!
//! Zone rules:
//! - [`Zone::Floating`] uses the device zone passed in. [`Zone::Fixed`] looks the
//!   name up in the time zone database and ignores the device zone.
//! - A local time that happens twice (the clocks go back) resolves to the first
//!   occurrence.
//! - A local time that never happens (the clocks go forward) resolves with the
//!   offset from before the gap. 02:30 in a gap from 02:00 to 03:00 fires at
//!   03:30. This is the behaviour RFC 5545 specifies.
//!
//! Two consequences callers must allow for:
//! - Resolving slots in order does not always give instants in order. In that
//!   gap, the 02:30 slot fires after the 03:00 slot. Choose the next alarm by
//!   instant, not by slot order.
//! - Two slots can resolve to the same instant. A window stepping hourly across
//!   that gap has slots at 02:00 and 03:00, and both resolve to 03:00.

use std::fmt;

use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::model::{Slot, Zone};

#[cfg(test)]
mod tests;

/// Why a slot could not be turned into an instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneError {
    /// The time zone database has no zone with this name.
    UnknownZone(String),
    /// The instant is outside the range that can be represented.
    OutOfRange,
}

impl fmt::Display for ZoneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownZone(name) => write!(f, "unknown time zone: {name}"),
            Self::OutOfRange => write!(f, "the instant is out of range"),
        }
    }
}

impl std::error::Error for ZoneError {}

/// The instant `slot` fires at in `zone`. `device` is the zone the device is in
/// now, used for [`Zone::Floating`].
pub fn instant_of(slot: Slot, zone: &Zone, device: &TimeZone) -> Result<Timestamp, ZoneError> {
    let fixed;
    let tz = match zone {
        Zone::Floating => device,
        Zone::Fixed(name) => {
            fixed = TimeZone::get(name).map_err(|_| ZoneError::UnknownZone(name.clone()))?;
            &fixed
        }
    };
    tz.to_timestamp(slot.0).map_err(|_| ZoneError::OutOfRange)
}
