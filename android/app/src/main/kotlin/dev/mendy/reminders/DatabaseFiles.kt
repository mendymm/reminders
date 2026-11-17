package dev.mendy.reminders

import android.content.Context
import android.content.pm.PackageManager
import dev.mendy.reminders.core.copyDatabase
import java.io.File

/**
 * Every app version keeps its own database file, `reminders-<versionCode>.db`. The first
 * time a version runs, it starts from a copy of the latest older version's file and
 * leaves that file alone. If an update goes wrong, the data it started from is still
 * there. Files are never removed.
 */
object DatabaseFiles {
    private const val DIRECTORY = "db"
    private val NAME = Regex("""reminders-(\d+)\.db""")

    /** The file the first builds used, before files carried a version code. */
    private const val LEGACY_NAME = "reminders.db"

    /** The version code of the running app, the one Gradle derives from the Cargo version. */
    fun versionCode(context: Context): Long = context.packageManager
        .getPackageInfo(context.packageName, PackageManager.PackageInfoFlags.of(0))
        .longVersionCode

    /**
     * The database for this version. The first call for a version copies the latest older
     * database into place. If there is none, the file does not exist yet and opening it
     * creates it. This reads and copies files, so it belongs off the UI thread.
     */
    fun forThisVersion(context: Context): File {
        val directory = File(context.filesDir, DIRECTORY).apply { mkdirs() }
        val version = versionCode(context)
        val target = File(directory, nameFor(version))
        if (target.exists()) return target

        // A copy that was cut short is not a database. It is never used, only replaced.
        val partial = File(directory, nameFor(version) + ".partial")
        partial.delete()

        val source = latestBefore(version, directory, File(context.filesDir, LEGACY_NAME))
        if (source != null) {
            copyDatabase(source.path, partial.path)
            check(partial.renameTo(target)) { "Could not move ${partial.name} to ${target.name}" }
        }
        return target
    }

    /** The database with the highest version code below [version], if there is one. */
    private fun latestBefore(version: Long, directory: File, legacy: File): File? {
        val versioned = directory.listFiles().orEmpty().mapNotNull { file ->
            NAME.matchEntire(file.name)?.let { it.groupValues[1].toLongOrNull()?.to(file) }
        }
        // The legacy file counts as version 0, so it is carried forward once.
        val candidates = if (legacy.exists()) versioned + (0L to legacy) else versioned
        return candidates
            .filter { (candidate, _) -> candidate < version }
            .maxByOrNull { (candidate, _) -> candidate }
            ?.second
    }

    private fun nameFor(version: Long): String = "reminders-$version.db"
}
