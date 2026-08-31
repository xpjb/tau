package app.tau

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonClassDiscriminator

const val TauProtocolVersion = 1

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
data class OpenSession(override val id: String, val sessionId: String) : ClientRequest

@Serializable
@SerialName("prompt")
data class Prompt(override val id: String, val sessionId: String, val text: String) : ClientRequest

@Serializable
@SerialName("abort")
data class Abort(override val id: String, val sessionId: String) : ClientRequest

@Serializable
@SerialName("close_session")
data class CloseSession(override val id: String, val sessionId: String) : ClientRequest

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

@Serializable
@SerialName("clone_session")
data class CloneSession(override val id: String, val sessionId: String) : ClientRequest

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
    val error: String? = null,
) : ServerMessage

@Serializable
@SerialName("sessions")
data class Sessions(val sessions: List<SessionSummary>) : ServerMessage

@Serializable
@SerialName("history")
data class History(val sessionId: String, val messages: List<ChatMessage>) : ServerMessage

@Serializable
@SerialName("session_state")
data class SessionState(
    val sessionId: String,
    val status: SessionStatus,
    val detail: String? = null,
) : ServerMessage

@Serializable
@SerialName("stream_reset")
data class StreamReset(val sessionId: String) : ServerMessage

@Serializable
@SerialName("stream_delta")
data class StreamDelta(val sessionId: String, val delta: String) : ServerMessage

@Serializable
@SerialName("stream_end")
data class StreamEnd(val sessionId: String) : ServerMessage

@Serializable
@SerialName("resync_required")
data object ResyncRequired : ServerMessage

@Serializable
data class SessionSummary(
    val id: String,
    val title: String,
    val status: SessionStatus,
    val detail: String? = null,
    val parentId: String? = null,
    val createdAtMs: Long,
    val updatedAtMs: Long,
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
data class ChatMessage(
    val entryId: String,
    val role: ChatRole,
    val text: String,
    val timestampMs: Long? = null,
)

@Serializable
enum class ChatRole {
    @SerialName("user") User,
    @SerialName("assistant") Assistant,
    @SerialName("system") System,
}

@Serializable
data class CrashReport(
    val schema: Int = 1,
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
