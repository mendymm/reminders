package dev.mendy.reminders

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Bundle
import android.text.format.DateFormat
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TimeInput
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import dev.mendy.reminders.core.Alert
import dev.mendy.reminders.core.AlertStyle
import dev.mendy.reminders.core.Cadence
import dev.mendy.reminders.core.Due
import dev.mendy.reminders.core.Plan
import dev.mendy.reminders.core.Recurrence
import dev.mendy.reminders.core.ReminderDraft
import dev.mendy.reminders.core.ReminderState
import dev.mendy.reminders.core.ReminderStore
import dev.mendy.reminders.core.Rule
import dev.mendy.reminders.core.TimesOfDay
import dev.mendy.reminders.core.Upcoming
import dev.mendy.reminders.core.When
import dev.mendy.reminders.core.Zone
import dev.mendy.reminders.core.newReminderId
import dev.mendy.reminders.core.newResolutionId
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.time.Instant
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

class MainActivity : ComponentActivity() {
    // Bumped each time the app comes to the front, so the screen reloads and the alarm
    // is set again. Reminders may have been resolved from a notification meanwhile.
    private val resumes = mutableIntStateOf(0)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            RemindersApp(refreshKey = resumes.intValue)
        }
    }

    override fun onResume() {
        super.onResume()
        resumes.intValue += 1
    }
}

/** A reminder and the slot waiting to be done, if any. */
private data class ReminderItem(val state: ReminderState, val pending: Upcoming?)

private fun loadItems(context: Context): List<ReminderItem> {
    val store = ReminderRuntime.store(context)
    val pending = store.upcoming(ReminderRuntime.callContext(), 1000u)
        .upcoming
        .associateBy { it.reminderId }
    return store.list().map { ReminderItem(it, pending[it.reminder.id]) }
}

private fun notificationsAllowed(context: Context): Boolean =
    context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
        PackageManager.PERMISSION_GRANTED

@Composable
private fun RemindersApp(refreshKey: Int) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var reminders by remember { mutableStateOf<List<ReminderItem>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var adding by remember { mutableStateOf(false) }
    var allowed by remember { mutableStateOf(notificationsAllowed(context)) }
    val askForNotifications = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted -> allowed = granted }

    // The store blocks, so everything that touches it runs off the UI thread.
    suspend fun refresh() {
        val result = withContext(Dispatchers.IO) { runCatching { loadItems(context) } }
        result
            .onSuccess {
                reminders = it
                error = null
            }
            .onFailure { error = it.message ?: it::class.simpleName }
    }

    /** Applies a change, sets the alarm again because the next slot may have moved, and reloads. */
    fun change(block: (ReminderStore) -> Unit) {
        scope.launch {
            val failure = withContext(Dispatchers.IO) {
                runCatching {
                    block(ReminderRuntime.store(context))
                    AlarmScheduler.reschedule(context)
                }.exceptionOrNull()
            }
            if (failure != null) error = failure.message ?: failure::class.simpleName
            refresh()
        }
    }

    LaunchedEffect(Unit) {
        if (!allowed) askForNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
    }
    LaunchedEffect(refreshKey) {
        withContext(Dispatchers.IO) { runCatching { AlarmScheduler.reschedule(context) } }
        refresh()
    }

    MaterialTheme {
        Scaffold(
            modifier = Modifier.fillMaxSize(),
            floatingActionButton = {
                FloatingActionButton(onClick = { adding = true }) { Text("+") }
            },
        ) { contentPadding ->
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(contentPadding)
                    .padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(text = "Reminders", style = MaterialTheme.typography.headlineMedium)
                if (!allowed) {
                    Text("Notifications are off, so reminders cannot alert you.")
                    TextButton(
                        onClick = {
                            askForNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
                        },
                    ) { Text("Allow notifications") }
                }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                if (reminders.isEmpty()) {
                    Text("No reminders yet. Tap + to add one.")
                }
                LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(reminders, key = { it.state.reminder.id }) { item ->
                        ReminderRow(
                            item = item,
                            onDone = {
                                val pending = item.pending ?: return@ReminderRow
                                change { store ->
                                    val ctx = ReminderRuntime.callContext()
                                    store.resolve(
                                        ctx,
                                        item.state.reminder.id,
                                        newResolutionId(ctx.nowMs),
                                        doneOutcome(item.state.reminder.plan, pending.slot),
                                        null,
                                    )
                                    Notifications.cancel(context, item.state.reminder.id)
                                }
                            },
                            onDelete = {
                                change { store ->
                                    val ctx = ReminderRuntime.callContext()
                                    val reminder = item.state.reminder
                                    store.delete(ctx, reminder.id, reminder.meta.revision)
                                    Notifications.cancel(context, reminder.id)
                                }
                            },
                        )
                    }
                }
            }
        }

        if (adding) {
            NewReminderDialog(
                onDismiss = { adding = false },
                onAdd = { draft ->
                    adding = false
                    change { store ->
                        val ctx = ReminderRuntime.callContext()
                        store.create(ctx, newReminderId(ctx.nowMs), draft)
                    }
                },
            )
        }
    }
}

@Composable
private fun ReminderRow(item: ReminderItem, onDone: () -> Unit, onDelete: () -> Unit) {
    val reminder = item.state.reminder
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(reminder.title, style = MaterialTheme.typography.titleMedium)
            Text(describe(reminder.plan), style = MaterialTheme.typography.bodyMedium)
            Text(describePending(item.pending), style = MaterialTheme.typography.bodyMedium)
            Row {
                TextButton(onClick = onDone, enabled = item.pending != null) { Text("Done") }
                TextButton(onClick = onDelete) { Text("Delete") }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NewReminderDialog(onDismiss: () -> Unit, onAdd: (ReminderDraft) -> Unit) {
    val context = LocalContext.current
    val now = remember { LocalTime.now() }
    var title by remember { mutableStateOf("") }
    var daily by remember { mutableStateOf(true) }
    var sticky by remember { mutableStateOf(false) }
    val time = rememberTimePickerState(
        initialHour = now.hour,
        initialMinute = now.minute,
        is24Hour = DateFormat.is24HourFormat(context),
    )

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("New reminder") },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OutlinedTextField(
                    value = title,
                    onValueChange = { title = it },
                    label = { Text("Title") },
                    singleLine = true,
                )
                Row {
                    FilterChip(
                        selected = !daily,
                        onClick = { daily = false },
                        label = { Text("Once") },
                    )
                    Spacer(Modifier.width(8.dp))
                    FilterChip(
                        selected = daily,
                        onClick = { daily = true },
                        label = { Text("Daily") },
                    )
                }
                TimeInput(state = time)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(checked = sticky, onCheckedChange = { sticky = it })
                    Text("Keep in the notification shade")
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = title.isNotBlank(),
                onClick = {
                    onAdd(newDraft(title, LocalTime.of(time.hour, time.minute), daily, sticky))
                },
            ) { Text("Add") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/**
 * A reminder at [time], once or every day, in whatever zone the device is in. It starts
 * today if the time is still ahead and tomorrow if not, so its first slot is never
 * already past.
 */
private fun newDraft(title: String, time: LocalTime, daily: Boolean, sticky: Boolean): ReminderDraft {
    val now = LocalDateTime.now()
    val firstDay = if (time.isAfter(now.toLocalTime())) {
        now.toLocalDate()
    } else {
        now.toLocalDate().plusDays(1)
    }
    val alert = Alert(style = AlertStyle.Notification(sticky = sticky), nudge = null)
    val plan = if (daily) {
        Plan.Ongoing(
            cadence = Cadence(
                recurrence = Recurrence(
                    startsOn = firstDay.toString(),
                    zone = Zone.Floating,
                    rule = Rule.Daily(every = 1u),
                    at = TimesOfDay.At(times = listOf(DateTimeFormatter.ISO_LOCAL_TIME.format(time))),
                    end = null,
                ),
                alert = alert,
            ),
        )
    } else {
        Plan.Once(
            due = Due(
                at = When(
                    local = DateTimeFormatter.ISO_LOCAL_DATE_TIME.format(LocalDateTime.of(firstDay, time)),
                    zone = Zone.Floating,
                ),
                alert = alert,
            ),
        )
    }
    return ReminderDraft(title = title.trim(), notes = "", plan = plan)
}

private fun describe(plan: Plan): String = when (plan) {
    is Plan.Once -> plan.due?.let { "Once, ${formatDateTime(it.at.local)}" } ?: "No time set"
    is Plan.Ongoing -> plan.cadence?.let { describe(it.recurrence) } ?: "Tracker"
}

private fun describe(recurrence: Recurrence): String {
    val rule = when (val rule = recurrence.rule) {
        is Rule.Daily -> if (rule.every == 1u) "Daily" else "Every ${rule.every} days"
        else -> "Repeats"
    }
    val at = when (val at = recurrence.at) {
        is TimesOfDay.At -> " at " + at.times.joinToString { formatTime(it) }
        else -> ""
    }
    return rule + at
}

private fun describePending(pending: Upcoming?): String {
    if (pending == null) return "Nothing due"
    val overdue = pending.atMs < Instant.now().toEpochMilli()
    return "Next: ${formatDateTime(pending.slot)}" + if (overdue) " (overdue)" else ""
}

private fun formatDateTime(text: String): String = runCatching {
    LocalDateTime.parse(text).format(DateTimeFormatter.ofLocalizedDateTime(FormatStyle.SHORT))
}.getOrDefault(text)

private fun formatTime(text: String): String = runCatching {
    LocalTime.parse(text).format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT))
}.getOrDefault(text)

@Preview(showBackground = true)
@Composable
private fun NewReminderPreview() {
    MaterialTheme {
        Text("Reminders")
    }
}
