package dev.mendy.reminders

import dev.mendy.reminders.core.Outcome
import dev.mendy.reminders.core.Plan

/**
 * The outcome that completes [slot] of a reminder with this [plan]. A one-off is
 * completed without naming a slot, a cadence completes the slot it names, and a
 * tracker has no slots.
 */
fun doneOutcome(plan: Plan, slot: String): Outcome = when (plan) {
    is Plan.Once -> Outcome.Done(slot = null)
    is Plan.Ongoing -> Outcome.Done(slot = if (plan.cadence != null) slot else null)
}
