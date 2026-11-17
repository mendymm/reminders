package dev.mendy.reminders

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import dev.mendy.reminders.core.AlertStyle
import dev.mendy.reminders.core.Upcoming
import java.time.LocalDateTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

/**
 * What a notification needs, carried in intent extras so the receiver that shows it does
 * not have to open the database.
 */
data class DueAlert(
    val reminderId: String,
    val title: String,
    val slot: String,
    val sticky: Boolean,
) {
    fun putInto(intent: Intent): Intent = intent
        .putExtra(REMINDER_ID, reminderId)
        .putExtra(TITLE, title)
        .putExtra(SLOT, slot)
        .putExtra(STICKY, sticky)

    companion object {
        private const val REMINDER_ID = "reminderId"
        private const val TITLE = "title"
        private const val SLOT = "slot"
        private const val STICKY = "sticky"

        fun from(upcoming: Upcoming): DueAlert = DueAlert(
            reminderId = upcoming.reminderId,
            title = upcoming.title,
            slot = upcoming.slot,
            // Only notifications are supported so far. An alarm shows as a plain one.
            sticky = (upcoming.alert.style as? AlertStyle.Notification)?.sticky ?: false,
        )

        fun from(intent: Intent): DueAlert? {
            val reminderId = intent.getStringExtra(REMINDER_ID) ?: return null
            val title = intent.getStringExtra(TITLE) ?: return null
            val slot = intent.getStringExtra(SLOT) ?: return null
            return DueAlert(reminderId, title, slot, intent.getBooleanExtra(STICKY, false))
        }
    }
}

object Notifications {
    private const val CHANNEL_ID = "reminders"

    /** Creating a channel that exists is a no-op, so this is safe to call every time. */
    fun ensureChannel(context: Context) {
        val channel = NotificationChannel(
            CHANNEL_ID,
            context.getString(R.string.notification_channel_name),
            NotificationManager.IMPORTANCE_HIGH,
        ).apply {
            description = context.getString(R.string.notification_channel_description)
        }
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    /**
     * Shows the notification for a slot that is due, replacing any earlier one for the
     * same reminder. A sticky notification stays until it is handled.
     */
    fun show(context: Context, alert: DueAlert) {
        ensureChannel(context)
        val id = notificationId(alert.reminderId)

        val open = PendingIntent.getActivity(
            context,
            id,
            Intent(context, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val done = PendingIntent.getBroadcast(
            context,
            id,
            alert.putInto(Intent(context, DoneReceiver::class.java)),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val icon = Icon.createWithResource(context, R.drawable.ic_launcher_monochrome)
        val notification = Notification.Builder(context, CHANNEL_ID)
            .setSmallIcon(icon)
            .setContentTitle(alert.title)
            .setContentText(dueText(alert.slot))
            .setCategory(Notification.CATEGORY_REMINDER)
            .setContentIntent(open)
            .setAutoCancel(!alert.sticky)
            .setOngoing(alert.sticky)
            .addAction(
                Notification.Action.Builder(icon, context.getString(R.string.action_done), done)
                    .build(),
            )
            .build()

        context.getSystemService(NotificationManager::class.java).notify(id, notification)
    }

    fun cancel(context: Context, reminderId: String) {
        context.getSystemService(NotificationManager::class.java).cancel(notificationId(reminderId))
    }

    private fun notificationId(reminderId: String): Int = reminderId.hashCode()

    private fun dueText(slot: String): String = runCatching {
        LocalDateTime.parse(slot).format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT))
    }.getOrDefault(slot)
}
