package dev.mendy.reminders

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent

/**
 * Keeps one alarm set, for the next slot to fire. When it fires, [AlarmReceiver] shows
 * the notification and sets the alarm for the one after. The core says which slot that
 * is, so nothing here knows about recurrence.
 */
object AlarmScheduler {
    private const val REQUEST_CODE = 1
    private const val ACTION = "dev.mendy.reminders.ALARM"

    /**
     * Sets the alarm for the next slot that has not fired yet, or clears it if there is
     * none. Call it after anything that can change which slot that is: a change to a
     * reminder, a resolution, a reboot, or a change of clock or time zone. It reads the
     * database, so it belongs off the UI thread.
     */
    fun reschedule(context: Context) {
        val store = ReminderRuntime.store(context)
        val next = store.alarms(ReminderRuntime.callContext(), 1u).upcoming.firstOrNull()
        val manager = context.getSystemService(AlarmManager::class.java)

        if (next == null) {
            existing(context)?.let(manager::cancel)
            return
        }

        val intent = DueAlert.from(next).putInto(intent(context))
        val pending = PendingIntent.getBroadcast(
            context,
            REQUEST_CODE,
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        if (manager.canScheduleExactAlarms()) {
            manager.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, next.atMs, pending)
        } else {
            manager.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, next.atMs, pending)
        }
    }

    private fun intent(context: Context): Intent =
        Intent(context, AlarmReceiver::class.java).setAction(ACTION)

    private fun existing(context: Context): PendingIntent? = PendingIntent.getBroadcast(
        context,
        REQUEST_CODE,
        intent(context),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_NO_CREATE,
    )
}
