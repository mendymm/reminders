package dev.mendy.reminders

import android.content.Context
import dev.mendy.reminders.core.CallContext
import dev.mendy.reminders.core.ReminderStore
import java.time.ZoneId

/** Process-wide access to the Rust store, shared by the UI and the receivers. */
object ReminderRuntime {
    private var store: ReminderStore? = null

    /**
     * The one store for this process, on this version's database file. The first call
     * may copy the previous version's database, so it belongs off the UI thread.
     */
    @Synchronized
    fun store(context: Context): ReminderStore {
        store?.let { return it }
        val file = DatabaseFiles.forThisVersion(context.applicationContext)
        return ReminderStore.open(file.path).also { store = it }
    }

    /** What the core needs to know about now: the time, and the zone the device is in. */
    fun callContext(): CallContext = CallContext(
        nowMs = System.currentTimeMillis(),
        deviceZone = ZoneId.systemDefault().id,
    )
}
