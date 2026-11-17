//! Checks on a plan that the types cannot express.

use jiff::tz::TimeZone;

use super::Invalid;
use crate::model::{End, Plan, Zone};

/// Rejects a plan whose zone is unknown to the time zone database, or whose
/// recurrence ends before it starts.
pub(super) fn validate_plan(plan: &Plan) -> Result<(), Invalid> {
    match plan {
        Plan::Once { due: Some(due) } => validate_zone(&due.when.zone),
        Plan::Once { due: None } | Plan::Ongoing { cadence: None } => Ok(()),
        Plan::Ongoing {
            cadence: Some(cadence),
        } => {
            let recurrence = &cadence.recurrence;
            validate_zone(&recurrence.zone)?;
            if let Some(End::Until(until)) = &recurrence.end
                && until.date() < recurrence.starts_on
            {
                return Err(Invalid::EndsBeforeStart);
            }
            Ok(())
        }
    }
}

fn validate_zone(zone: &Zone) -> Result<(), Invalid> {
    match zone {
        Zone::Floating => Ok(()),
        Zone::Fixed(name) => TimeZone::get(name)
            .map(|_| ())
            .map_err(|_| Invalid::UnknownZone(name.clone())),
    }
}
