package app.tau

import kotlinx.serialization.EncodeDefault
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonClassDiscriminator

const val TauProtocolVersion = 4

val TauJson = Json {
    classDiscriminator = "type"
    ignoreUnknownKeys = true
    encodeDefaults = false
}

@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("type")
sealed interface ClientRequest {
    val id: String
}

@Serializable
@SerialName("list_sessions")
data class ListSessions(override val id: String) : ClientRequest

@Serializable
@SerialName("create_session")
data class CreateSession(override val id: String) : ClientRequest

@Serializable
@SerialName("open_session")
data class OpenSession(override val id: String, val sessionId: String, val requests: List<String> = emptyList(), val streams: List<String> = emptyList()) : ClientRequest

@Serializable
@SerialName("get_history")
data class GetHistory(override val id: String, val sessionId: String, val generation: String, val before: String) : ClientRequest

@Serializable
@SerialName("get_commands")
data class GetCommands(override val id: String, val sessionId: String) : ClientRequest

@Serializable
@SerialName("prompt")
data class Prompt(override val id: String, val sessionId: String, val text: String) : ClientRequest

@Serializable
@SerialName("queue_control")
data class ControlQueue(override val id: String, val sessionId: String, val generation: String, val operation: QueueOperation) : ClientRequest

@Serializable
@SerialName("clone_session")
data class CloneSession(override val id: String, val sessionId: String) : ClientRequest

@Serializable
@SerialName("extension_ui_response")
data class RespondExtensionUi(
    override val id: String,
    val sessionId: String,
    val requestId: String,
    val value: String? = null,
    val confirmed: Boolean? = null,
    val cancelled: Boolean = false,
) : ClientRequest

@Serializable
@SerialName("abort")
data class Abort(override val id: String, val sessionId: String) : ClientRequest

@Serializable
@SerialName("delete_session")
data class DeleteSession(override val id: String, val sessionId: String) : ClientRequest

@Serializable
@SerialName("rename_session")
data class RenameSession(
    override val id: String,
    val sessionId: String,
    val title: String,
) : ClientRequest

@Serializable
@SerialName("fork_session")
data class ForkSession(
    override val id: String,
    val sessionId: String,
    val entryId: String,
) : ClientRequest

@OptIn(ExperimentalSerializationApi::class)
@Serializable
@JsonClassDiscriminator("type")
sealed interface ServerMessage

@Serializable
@SerialName("hello")
data class Hello(val protocolVersion: Int, val daemonVersion: String) : ServerMessage

@Serializable
@SerialName("response")
data class Response(
    val requestId: String,
    val ok: Boolean,
    val sessionId: String? = null,
    val draft: String? = null,
    val disposition: String? = null,
    val uncertain: Boolean = false,
    val outcome: String? = null,
    val notice: String? = null,
    val error: String? = null,
) : ServerMessage

@Serializable
@SerialName("commands")
data class Commands(
    val sessionId: String,
    val commands: List<SlashCommand>,
) : ServerMessage

@Serializable
@SerialName("extension_ui")
data class ExtensionUi(
    val sessionId: String,
    val request: ExtensionUiRequest,
) : ServerMessage

@Serializable
@SerialName("extension_error")
data class ExtensionError(val sessionId: String, val error: String) : ServerMessage

@Serializable
@SerialName("sessions")
data class Sessions(val sessions: List<SessionSummary>) : ServerMessage

@Serializable
@SerialName("transcript_snapshot")
data class TranscriptSnapshot(val sessionId: String, val snapshot: TranscriptCut) : ServerMessage

@Serializable
@SerialName("transcript_page")
data class TranscriptPage(val requestId: String, val sessionId: String, val generation: String, val cursor: String, val page: HistoryPage) : ServerMessage

@Serializable
@SerialName("transcript_update")
data class TranscriptUpdate(val sessionId: String, val generation: String, val sequence: Long, val change: TranscriptChange) : ServerMessage

@Serializable
@SerialName("session_state")
data class SessionState(
    val sessionId: String,
    val status: SessionStatus,
    val detail: String? = null,
    val contextUsage: ContextUsage? = null,
) : ServerMessage

@Serializable
@SerialName("resync_required")
data class ResyncRequired(val sessionId: String? = null) : ServerMessage

@Serializable
data class SlashCommand(
    val name: String,
    val description: String? = null,
    val source: SlashCommandSource,
    val argumentHint: String? = null,
    val arguments: List<SlashCommandArgument> = emptyList(),
)

@Serializable
enum class SlashCommandSource {
    @SerialName("extension") Extension,
    @SerialName("prompt") Prompt,
    @SerialName("skill") Skill,
    @SerialName("builtin") Builtin,
}

@Serializable
data class SlashCommandArgument(
    val value: String,
    val description: String? = null,
)

@Serializable
data class ExtensionUiRequest(
    val id: String,
    val method: String,
    val title: String? = null,
    val message: String? = null,
    val options: List<String> = emptyList(),
    val placeholder: String? = null,
    val prefill: String? = null,
    val timeout: Long? = null,
    val notifyType: String? = null,
    val statusKey: String? = null,
    val statusText: String? = null,
    val widgetKey: String? = null,
    val widgetLines: List<String> = emptyList(),
    val widgetPlacement: String? = null,
    val text: String? = null,
)

@Serializable
data class SessionModel(
    val provider: String,
    val modelId: String,
)

@Serializable
data class ContextUsage(val tokens: Long? = null, val contextWindow: Long)

@Serializable
data class SessionSummary(
    val id: String,
    val title: String,
    val status: SessionStatus,
    val detail: String? = null,
    val model: SessionModel? = null,
    val parentId: String? = null,
    val createdAtMs: Long,
    val updatedAtMs: Long,
    val contextUsage: ContextUsage? = null,
)

@Serializable
enum class SessionStatus {
    @SerialName("sleeping") Sleeping,
    @SerialName("starting") Starting,
    @SerialName("idle") Idle,
    @SerialName("running") Running,
    @SerialName("error") Error,
}

@Serializable
data class UploadedFile(
    val name: String,
    val path: String,
    val size: Long,
)

@Serializable
data class ChatAttachment(
    val kind: AttachmentKind,
    val fileName: String,
    val caption: String? = null,
    val size: Long? = null,
)

@Serializable
enum class AttachmentKind {
    @SerialName("image") Image,
    @SerialName("file") File,
}

@OptIn(ExperimentalSerializationApi::class)
@Serializable
data class CrashReport(
    @EncodeDefault val schema: Int = 1,
    val reportId: String,
    val platform: String,
    val appVersion: String,
    val osVersion: String,
    val thread: String,
    val exceptionClass: String,
    val stack: List<CrashFrame>,
)

@Serializable
data class CrashFrame(
    val className: String,
    val methodName: String,
    val fileName: String? = null,
    val lineNumber: Int,
)
