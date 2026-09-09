package app.tau

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonClassDiscriminator
import okio.ByteString.Companion.encodeUtf8
import kotlin.uuid.Uuid

val ConnectionSettings.identity: String
    get() = "${serverUrl.trim().trimEnd('/')}\u0000$token".encodeUtf8().sha256().hex()

fun newRequestId(): String = Uuid.random().toString()

@Serializable
data class EventOrigin(
    val requestId: String? = null,
    val requestRevision: Long? = null,
    val streamId: String? = null,
)

@Serializable
enum class EventPhase {
    @SerialName("saved") Saved,
    @SerialName("live") Live,
    @SerialName("interrupted") Interrupted,
}

@Serializable
enum class EventRole {
    @SerialName("user") User,
    @SerialName("assistant") Assistant,
    @SerialName("tool") Tool,
    @SerialName("system") System,
}

@Serializable
enum class EventKind {
    @SerialName("text") Text,
    @SerialName("thinking") Thinking,
    @SerialName("tool") Tool,
    @SerialName("image") Image,
    @SerialName("hidden") Hidden,
}

@Immutable
@Serializable
data class TranscriptEvent(
    val id: String,
    val order: Long,
    val entryId: String,
    val phase: EventPhase = EventPhase.Saved,
    val origin: EventOrigin = EventOrigin(),
    val role: EventRole,
    val kind: EventKind,
    val text: String = "",
    val timestamp: String? = null,
    val timestampMs: Long? = null,
    val toolCallId: String? = null,
    val toolName: String? = null,
    val stopReason: String? = null,
    val errorMessage: String? = null,
    val isError: Boolean = false,
    val attachment: ChatAttachment? = null,
)

@Serializable
data class QueueRef(val requestId: String, val revision: Long)

@Serializable
data class QueuedRequest(
    val requestId: String,
    val revision: Long,
    val kind: String,
    val text: String,
    val images: Int = 0,
    val timestampMs: Long? = null,
)

@Serializable
data class QueueControl(
    val commandId: String,
    val runId: String? = null,
    val action: String,
    val boundary: String? = null,
    val requests: List<QueueRef> = emptyList(),
    val status: String,
    val detail: String? = null,
)

@Serializable
data class QueueState(
    val available: Boolean = false,
    val requests: List<QueuedRequest> = emptyList(),
    val runId: String? = null,
    val paused: Boolean = false,
    val control: QueueControl? = null,
    val capabilities: List<String> = emptyList(),
    val boundaries: List<String> = emptyList(),
)

@Serializable
data class TranscriptCut(
    val generation: String,
    val sequence: Long,
    val events: List<TranscriptEvent>,
    val queue: QueueState,
    val before: Long? = null,
    val delivered: List<String> = emptyList(),
)

@Serializable
data class HistoryPage(val events: List<TranscriptEvent>, val before: Long? = null)

@Serializable
data class TranscriptPatch(val generation: String, val sequence: Long, val change: TranscriptChange)

@Serializable
data class TextDelta(val eventId: String, val text: String)

@Serializable
data class TranscriptChange(
    val events: List<TranscriptEvent> = emptyList(),
    val removed: List<String> = emptyList(),
    val delta: TextDelta? = null,
    val queue: QueueState? = null,
    val delivered: List<String> = emptyList(),
)

@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("type")
sealed interface QueueOperation {
    @Serializable @SerialName("edit") data class Edit(val requestId: String, val revision: Long, val text: String) : QueueOperation
    @Serializable @SerialName("delete") data class Delete(val requestId: String, val revision: Long) : QueueOperation
    @Serializable @SerialName("prefix") data class Prefix(val runId: String?, val requests: List<QueueRef>, val boundary: String) : QueueOperation
    @Serializable @SerialName("pause") data class Pause(val runId: String?, val boundary: String) : QueueOperation
    @Serializable @SerialName("resume") data class Resume(val runId: String?) : QueueOperation
    @Serializable @SerialName("cancel") data class Cancel(val controlId: String) : QueueOperation
}

@Serializable
enum class SendStatus(val label: String) {
    Preparing("Preparing attachments"), Sending("Sending"), Accepted("Accepted by Pi"),
    Queued("Queued"), Unconfirmed("Delivery unconfirmed"), Rejected("Not sent"),
}

@Serializable
data class DraftFile(val id: String, val name: String, val size: Long)

@Serializable
data class PendingSend(
    val requestId: String,
    val text: String,
    val wireText: String? = null,
    val files: List<DraftFile> = emptyList(),
    val revision: Long = 0,
    val status: SendStatus = SendStatus.Sending,
    val detail: String? = null,
)

@Serializable
data class PendingControl(
    val commandId: String,
    val generation: String,
    val operation: QueueOperation,
    val status: String = "sending",
    val detail: String? = null,
)

@Serializable
data class ScrollPosition(val key: String? = null, val offset: Int = 0, val follow: Boolean = true)

internal data class ChatPosition(
    val generation: String = "",
    val sequence: Long = 0,
    val queue: QueueState = QueueState(),
)

internal const val HistoryPageEvents = 50
internal const val HistoryPageBytes = 256 * 1024
internal val TranscriptEvent.pageBytes: Long get() = 512L + text.length * 3L

data class ChatKey(val connection: String, val session: String)
data class AttachmentDownloadKey(val sessionId: String, val entryId: String)
data class StoredConnection(
    val sessions: List<SessionSummary>,
    val selected: String?,
    val readAt: Map<String, Long> = emptyMap(),
    val downloads: Map<AttachmentDownloadKey, SavedDownload> = emptyMap(),
)

@Stable
class EventRow internal constructor(event: TranscriptEvent) {
    val key: String = event.id
    var event: TranscriptEvent by mutableStateOf(event)
        internal set
}

@Stable
class RetainedChat internal constructor(val key: ChatKey) {
    internal val byId = linkedMapOf<String, EventRow>()
    internal var position: ChatPosition by mutableStateOf(ChatPosition())
    internal val mutableRows = mutableStateListOf<EventRow>()
    var before: Long? by mutableStateOf(null)
        internal set
    internal val mutablePending = mutableStateListOf<PendingSend>()
    internal val mutableControls = mutableStateListOf<PendingControl>()
    internal val mutableFiles = mutableStateListOf<DraftFile>()
    internal val mutablePreferences = mutableStateMapOf<String, String>()
    val rows: List<EventRow> get() = mutableRows
    val pending: List<PendingSend> get() = mutablePending
    val controls: List<PendingControl> get() = mutableControls
    val files: List<DraftFile> get() = mutableFiles
    val preferences: Map<String, String> get() = mutablePreferences
    var synchronized: Boolean by mutableStateOf(false)
        internal set
    val queue: QueueState by derivedStateOf { position.queue }

    internal fun merge(events: Collection<TranscriptEvent>, removed: Collection<String> = emptyList(), replace: Boolean = false) {
        var membershipChanged = false
        for (id in removed) if (byId.remove(id) != null) membershipChanged = true
        if (replace) {
            val ids = events.mapTo(mutableSetOf()) { it.id }
            membershipChanged = byId.keys.retainAll(ids) || membershipChanged
        }
        for (event in events) {
            val row = byId[event.id]
            if (row == null) { byId[event.id] = EventRow(event); membershipChanged = true }
            else { membershipChanged = membershipChanged || row.event.order != event.order; row.event = event }
        }
        if (membershipChanged) {
            val ordered = byId.values.sortedBy { it.event.order }
            mutableRows.clear(); mutableRows.addAll(ordered)
        }
    }
}
