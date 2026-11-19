package sh.zuse.core

import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.TextStyle
import java.time.temporal.ChronoUnit
import java.util.Locale

enum class ChatStatus(val label: String?) {
    Idle(null),
    Working("Working"),
    NeedsInput("Needs input"),
    Failed("Failed");

    companion object {
        /** Engine runtime states and cloud runtime states, mapped to what a person needs to know. */
        fun from(value: String?): ChatStatus =
            when (value?.lowercase()) {
                "working",
                "running",
                "busy" -> Working
                "awaitinginput",
                "awaiting_input",
                "needsinput" -> NeedsInput
                "errored",
                "failed",
                "error" -> Failed
                else -> Idle
            }
    }
}

/** One chat in the home list, from any place a chat can run (a computer or Zuse Cloud). */
data class ChatRow(
    val key: String,
    val chatId: String,
    val placeKey: String,
    val placeLabel: String,
    val projectKey: String,
    val projectName: String?,
    val title: String,
    val preview: String? = null,
    val status: ChatStatus = ChatStatus.Idle,
    val branch: String? = null,
    val activityAt: Long? = null,
    val archived: Boolean = false,
)

/** Which chats the home list shows. */
enum class ArchiveFilter(val label: String) {
    Active("Active"),
    Archived("Archived"),
    All("All chats");

    fun shows(row: ChatRow) =
        when (this) {
            Active -> !row.archived
            Archived -> row.archived
            All -> true
        }
}

data class ChatGroup(
    val key: String,
    val title: String,
    /** The computer this group lives on, only when chats from several places are listed. */
    val place: String?,
    val rows: List<ChatRow>,
)

object ChatList {
    const val NO_PROJECT = "No project"

    /** Groups chats by project, newest activity first, filtered by place and a search query. */
    /** Must match the view model's place key for Zuse Cloud. */
    const val CLOUD_PLACE = "cloud"

    fun groups(
        rows: List<ChatRow>,
        query: String = "",
        place: String? = null,
        archive: ArchiveFilter = ArchiveFilter.Active,
    ): List<ChatGroup> {
        val needle = query.trim()
        val visible =
            rows
                .filter { place == null || it.placeKey == place }
                .filter(archive::shows)
                .filter {
                    needle.isEmpty() ||
                        it.title.contains(needle, true) ||
                        it.preview?.contains(needle, true) == true ||
                        it.branch?.contains(needle, true) == true ||
                        (it.projectName ?: NO_PROJECT).contains(needle, true)
                }
                .distinctBy { it.key }
        // Cloud chats are ordinary chats: they join their repository's group (marked by an icon on
        // the row) and never make the list look like it spans several computers.
        val computers = visible.filter { it.placeKey != CLOUD_PLACE }
        val severalPlaces = computers.map { it.placeKey }.distinct().size > 1
        val groupOf = HashMap<String, String>()
        computers.forEach { row ->
            val name = (row.projectName ?: NO_PROJECT).lowercase()
            groupOf.putIfAbsent(name, "${row.placeKey}|${row.projectKey}")
        }
        fun key(row: ChatRow) =
            if (row.placeKey != CLOUD_PLACE) "${row.placeKey}|${row.projectKey}"
            else groupOf[(row.projectName ?: NO_PROJECT).lowercase()] ?: "$CLOUD_PLACE|${(row.projectName ?: NO_PROJECT).lowercase()}"
        val newestFirst =
            compareByDescending<ChatRow> { it.activityAt ?: Long.MIN_VALUE }
                .thenBy { it.title.lowercase() }
        return visible
            .groupBy(::key)
            .map { (key, chats) ->
                val first = chats.firstOrNull { it.placeKey != CLOUD_PLACE } ?: chats.first()
                ChatGroup(
                    key,
                    first.projectName?.takeIf { it.isNotBlank() } ?: NO_PROJECT,
                    if (severalPlaces && first.placeKey != CLOUD_PLACE) first.placeLabel else null,
                    chats.sortedWith(newestFirst),
                )
            }
            .sortedWith(
                compareByDescending<ChatGroup> {
                        it.rows.first().activityAt ?: Long.MIN_VALUE
                    }
                    .thenBy { it.title.lowercase() }
            )
    }

    /** The line under a chat title: what needs attention first, then the latest message. */
    fun detail(row: ChatRow): String? =
        row.status.label?.let { if (row.status == ChatStatus.Working) "Working…" else it }
            ?: row.preview?.lineSequence()?.map { it.trim() }?.firstOrNull { it.isNotEmpty() }

    fun parseTime(value: String?): Long? =
        value?.let { runCatching { Instant.parse(it).toEpochMilli() }.getOrNull() }

    /** Compact chat-list time: now, 5m, 3h, Yesterday, Mon, 12 Nov, 12 Nov 2025. */
    fun relativeTime(
        at: Long?,
        now: Long = System.currentTimeMillis(),
        zone: ZoneId = ZoneId.systemDefault(),
        locale: Locale = Locale.getDefault(),
    ): String {
        if (at == null) return ""
        val elapsed = now - at
        if (elapsed < 60_000) return "now"
        if (elapsed < 3_600_000) return "${elapsed / 60_000}m"
        val then = Instant.ofEpochMilli(at).atZone(zone)
        val today = Instant.ofEpochMilli(now).atZone(zone).toLocalDate()
        val days = ChronoUnit.DAYS.between(then.toLocalDate(), today)
        return when {
            days <= 0L -> "${elapsed / 3_600_000}h"
            days == 1L -> "Yesterday"
            days < 7L -> then.dayOfWeek.getDisplayName(TextStyle.SHORT, locale)
            then.year == today.year -> DateTimeFormatter.ofPattern("d MMM", locale).format(then)
            else -> DateTimeFormatter.ofPattern("d MMM yyyy", locale).format(then)
        }
    }
}

/** How a new chat checks out its project. A chat without a project can only work locally. */
enum class CheckoutMode(val label: String) {
    Local("Work locally"),
    Worktree("New worktree"),
    Branch("Existing branch");

    companion object {
        fun available(projectId: String?): List<CheckoutMode> =
            if (projectId == null || projectId == CanonicalWorkspace.PROJECTLESS) listOf(Local)
            else entries

        /** The last choice for this project when it is still possible, otherwise working locally. */
        fun default(projectId: String?, remembered: Map<String, String>): CheckoutMode {
            val previous =
                projectId
                    ?.let(remembered::get)
                    ?.let { name -> entries.firstOrNull { it.name == name } }
            return previous?.takeIf { it in available(projectId) } ?: Local
        }
    }
}

/**
 * Whether a failed request may have reached the host. Only those are uncertain: a request the
 * client refused to send, or one the host answered with a rejection, certainly did not run.
 */
object RequestOutcome {
    fun uncertain(failure: Throwable): Boolean =
        failure !is RpcRequestFailure && failure !is IllegalArgumentException
}
