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
data class EntryOrigin(
    val requestId: String? = null,
    val requestRevision: Long? = null,
    val streamId: String? = null,
)

@Serializable
enum class EntryPhase {
    @SerialName("saved") Saved,
    @SerialName("live") Live,
    @SerialName("interrupted") Interrupted,
}

@Serializable
enum class EntryRole {
    @SerialName("user") User,
    @SerialName("assistant") Assistant,
    @SerialName("tool") Tool,
    @SerialName("system") System,
}

@Serializable
enum class ContentKind {
    @SerialName("text") Text,
    @SerialName("thinking") Thinking,
    @SerialName("tool") Tool,
    @SerialName("image") Image,
    @SerialName("hidden") Hidden,
}

@Immutable
@Serializable
data class EntryContent(
    val kind: ContentKind,
    val text: String = "",
    val toolCallId: String? = null,
    val toolName: String? = null,
)

@Immutable
@Serializable
data class TranscriptEntry(
    val id: String,
    val parentId: String? = null,
    val entryType: String = "message",
    val phase: EntryPhase = EntryPhase.Saved,
    val origin: EntryOrigin = EntryOrigin(),
    val role: EntryRole? = null,
    val timestamp: String? = null,
    val timestampMs: Long? = null,
    val content: List<EntryContent> = emptyList(),
    val toolCallId: String? = null,
    val toolName: String? = null,
    val stopReason: String? = null,
    val errorMessage: String? = null,
    val isError: Boolean = false,
    val attachment: ChatAttachment? = null,
) {
    val displayKey: String get() = origin.streamId?.let { "stream:$it" }
        ?: origin.requestId?.takeIf { role == EntryRole.User }?.let { "request:$it" } ?: "entry:$id"
}

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
    val head: String? = null,
    val entries: List<TranscriptEntry>,
    val queue: QueueState,
)

@Serializable
data class TranscriptPatch(val generation: String, val sequence: Long, val change: TranscriptChange)

@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("type")
sealed interface TranscriptChange {
    @Serializable @SerialName("entry") data class Entry(val entry: TranscriptEntry) : TranscriptChange
    @Serializable @SerialName("block") data class Block(val entryId: String, val index: Int, val content: EntryContent) : TranscriptChange
    @Serializable @SerialName("delta") data class Delta(val entryId: String, val index: Int, val delta: String) : TranscriptChange
    @Serializable @SerialName("head") data class Head(val head: String? = null) : TranscriptChange
    @Serializable @SerialName("queue") data class Queue(val queue: QueueState) : TranscriptChange
    @Serializable @SerialName("interrupted") data object Interrupted : TranscriptChange
}

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

@Serializable
internal data class StoredPosition(
    val generation: String = "",
    val sequence: Long = 0,
    val head: String? = null,
    val queue: QueueState = QueueState(),
)

data class ChatKey(val connection: String, val session: String)

data class StoredConnection(val sessions: List<SessionSummary>, val selected: String?)

@Stable
class EntryRow internal constructor(entry: TranscriptEntry) {
    val key: String = entry.displayKey
    var entry: TranscriptEntry by mutableStateOf(entry)
        internal set
}

@Stable
class RetainedChat internal constructor(val key: ChatKey) {
    internal val byId = linkedMapOf<String, EntryRow>()
    internal val branch = mutableSetOf<String>()
    internal val visibleKeys = mutableSetOf<String>()
    internal var position: StoredPosition by mutableStateOf(StoredPosition())
    internal val mutableRows = mutableStateListOf<EntryRow>()
    internal val mutablePending = mutableStateListOf<PendingSend>()
    internal val mutableControls = mutableStateListOf<PendingControl>()
    internal val mutableFiles = mutableStateListOf<DraftFile>()
    internal val mutablePreferences = mutableStateMapOf<String, String>()
    val rows: List<EntryRow> get() = mutableRows
    val pending: List<PendingSend> get() = mutablePending
    val controls: List<PendingControl> get() = mutableControls
    val files: List<DraftFile> get() = mutableFiles
    val preferences: Map<String, String> get() = mutablePreferences
    var synchronized: Boolean by mutableStateOf(false)
        internal set
    val queue: QueueState by derivedStateOf { position.queue }

    internal fun rebuildRows() {
        branch.clear()
        val path = mutableListOf<EntryRow>()
        var cursor = position.head
        while (cursor != null) {
            check(branch.add(cursor)) { "Cyclic transcript branch" }
            val row = checkNotNull(byId[cursor]) { "Missing transcript parent" }
            path.add(row)
            cursor = row.entry.parentId
        }
        val provisional = byId.values.filter { it.entry.phase != EntryPhase.Saved }.groupBy { it.entry.parentId }
        mutableRows.clear()
        visibleKeys.clear()
        for (row in path.asReversed()) {
            if (row.entry.role != null) { mutableRows.add(row); visibleKeys.add(row.key) }
            for (tail in provisional[row.entry.id].orEmpty()) {
                if (tail.entry.role != null) { mutableRows.add(tail); visibleKeys.add(tail.key) }
            }
        }
        for (row in provisional[null].orEmpty()) {
            if (row.entry.role != null) { mutableRows.add(row); visibleKeys.add(row.key) }
        }
    }
}
