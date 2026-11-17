package dev.mendy.reminders

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import dev.mendy.reminders.core.ReminderException
import dev.mendy.reminders.core.newResolutionId
import kotlin.concurrent.thread

/**
 * Runs [block] off the main thread and keeps the receiver alive until it is done. A
 * receiver is only given a few seconds, and these touch the database.
 */
private fun BroadcastReceiver.inBackground(block: () -> Unit) {
    val result = goAsync()
    thread {
        try {
            block()
        } finally {
            result.finish()
        }
    }
}

/**
 * The alarm for a slot went off. The notification comes first and comes from the intent
 * alone. Setting the next alarm needs the database, so it follows in the background.
 */
class AlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val alert = DueAlert.from(intent) ?: return
        Notifications.show(context, alert)
        inBackground { AlarmScheduler.reschedule(context) }
    }
}

/** The "Done" action on a notification: resolve the slot it was for. */
class DoneReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val alert = DueAlert.from(intent) ?: return
        Notifications.cancel(context, alert.reminderId)
        inBackground {
            val store = ReminderRuntime.store(context)
            val ctx = ReminderRuntime.callContext()
            try {
                val plan = store.get(alert.reminderId).reminder.plan
                store.resolve(
                    ctx,
                    alert.reminderId,
                    newResolutionId(ctx.nowMs),
                    doneOutcome(plan, alert.slot),
                    null,
                )
            } catch (_: ReminderException) {
                // Already handled, or the reminder is gone or archived. Nothing to record.
            }
            AlarmScheduler.reschedule(context)
        }
    }
}

/** After a reboot, an update, or a change of clock or time zone, set the alarm again. */
class RescheduleReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        inBackground { AlarmScheduler.reschedule(context) }
    }
}
