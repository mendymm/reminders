package dev.mendy.reminders

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.tooling.preview.Preview
import dev.mendy.reminders.core.Alert
import dev.mendy.reminders.core.AlertStyle
import dev.mendy.reminders.core.CallContext
import dev.mendy.reminders.core.Due
import dev.mendy.reminders.core.Plan
import dev.mendy.reminders.core.ReminderDraft
import dev.mendy.reminders.core.ReminderStore
import dev.mendy.reminders.core.When
import dev.mendy.reminders.core.Zone
import dev.mendy.reminders.core.newReminderId
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.time.Instant
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.temporal.ChronoUnit

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            RemindersApp()
        }
    }
}

@Composable
private fun RemindersApp() {
    val databasePath = File(LocalContext.current.filesDir, "reminders.db").path
    var rustStatus by remember { mutableStateOf("Opening the store...") }
    LaunchedEffect(databasePath) {
        // The store blocks, so it runs off the UI thread.
        rustStatus = withContext(Dispatchers.IO) {
            runCatching { smokeTest(databasePath) }.getOrElse { error ->
                "Rust call failed: ${error.message ?: error::class.simpleName}"
            }
        }
    }

    MaterialTheme {
        Scaffold(modifier = Modifier.fillMaxSize()) { contentPadding ->
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(contentPadding),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Text(
                    text = "Reminders",
                    style = MaterialTheme.typography.headlineMedium,
                )
                Text(
                    text = rustStatus,
                    style = MaterialTheme.typography.bodyLarge,
                )
            }
        }
    }
}

@Preview(showBackground = true)
@Composable
private fun RemindersPreview() {
    MaterialTheme {
        Text("Rust store: 1 reminder(s)")
    }
}


/** Opens the store, adds one reminder the first time, and describes what it holds. */
private fun smokeTest(databasePath: String): String {
    ReminderStore.open(databasePath).use { store ->
        val now = System.currentTimeMillis()
        val ctx = CallContext(nowMs = now, deviceZone = ZoneId.systemDefault().id)
        if (store.list().isEmpty()) {
            val due = LocalDateTime.now().plusMinutes(1).truncatedTo(ChronoUnit.SECONDS)
            store.create(ctx, newReminderId(now), firstReminder(due))
        }
        val reminders = store.list()
        val next = store.upcoming(ctx, 1u).upcoming.firstOrNull()
        val nextText = next?.let { "${it.title} at ${Instant.ofEpochMilli(it.atMs)}" }
            ?: "nothing upcoming"
        return "Rust store: ${reminders.size} reminder(s)\nNext: $nextText"
    }
}

private fun firstReminder(due: LocalDateTime): ReminderDraft = ReminderDraft(
    title = "First reminder from Rust",
    notes = "",
    plan = Plan.Once(
        due = Due(
            at = When(
                local = DateTimeFormatter.ISO_LOCAL_DATE_TIME.format(due),
                zone = Zone.Floating,
            ),
            alert = Alert(style = AlertStyle.Notification(sticky = false), nudge = null),
        ),
    ),
)
