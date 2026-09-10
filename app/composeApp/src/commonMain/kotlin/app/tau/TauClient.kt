package app.tau

import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.network.sockets.ConnectTimeoutException
import io.ktor.client.network.sockets.SocketTimeoutException
import io.ktor.client.plugins.HttpRequestTimeoutException
import io.ktor.client.statement.bodyAsChannel
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.HttpTimeoutConfig
import io.ktor.client.plugins.timeout
import io.ktor.client.plugins.websocket.DefaultClientWebSocketSession
import io.ktor.client.plugins.websocket.WebSockets
import io.ktor.client.plugins.websocket.WebSocketException
import io.ktor.client.plugins.websocket.webSocket
import io.ktor.client.request.header
import io.ktor.client.request.post
import io.ktor.client.request.prepareGet
import io.ktor.client.request.setBody
import io.ktor.client.request.url
import io.ktor.http.ContentType
import io.ktor.http.HttpHeaders
import io.ktor.http.contentType
import io.ktor.http.isSuccess
import io.ktor.http.encodeURLPathPart
import io.ktor.utils.io.readAvailable
import kotlinx.io.IOException
import io.ktor.util.network.UnresolvedAddressException
import io.ktor.websocket.Frame
import io.ktor.websocket.readText
import io.ktor.websocket.send
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.cancel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import okio.ByteString.Companion.encodeUtf8
import okio.FileSystem
import okio.Path.Companion.toPath
import okio.buffer

class TauConnectionException(message: String, cause: Throwable? = null) : Exception(message, cause)

internal fun Throwable.isConnectionFailure(): Boolean = generateSequence(this) { it.cause }.any {
    it is TauConnectionException || it is IOException || it is UnresolvedAddressException || it is WebSocketException
}

sealed class AttachmentFailure(val message: String) {
    data object NotLocal : AttachmentFailure("Not saved on this device. Connect and retry.")
    data object TooLarge : AttachmentFailure("Attachment exceeds Tau's download limit")
    data object TimedOut : AttachmentFailure("Timed out")
    data object Interrupted : AttachmentFailure("Download interrupted")
    data object Cancelled : AttachmentFailure("Download cancelled")
    data class Http(val status: Int) : AttachmentFailure("HTTP $status")
}

class AttachmentDownloadException(val failure: AttachmentFailure, cause: Throwable? = null) : Exception(failure.message, cause)

class TauClient(private val attachmentDirectory: () -> String = { PlatformServices.attachmentDirectory }) {
    private val client = HttpClient(platformHttpEngine()) {
        followRedirects = false
        install(HttpTimeout)
        install(WebSockets)
    }
    private val socketGate = Mutex()
    private val attachmentGate = Semaphore(2)
    private var socket: DefaultClientWebSocketSession? = null
    private var socketId = 0L

    suspend fun run(settings: ConnectionSettings, onMessages: suspend (List<ServerMessage>, Long) -> Unit) {
        val baseUrl = settings.serverUrl.trim().trimEnd('/')
        require(baseUrl.startsWith("http://") || baseUrl.startsWith("https://")) {
            "Server URL must start with http:// or https://"
        }
        val websocketUrl = when {
            baseUrl.startsWith("https://") -> {
                "wss://${baseUrl.removePrefix("https://")}/v1/ws"
            }
            else -> "ws://${baseUrl.removePrefix("http://")}/v1/ws"
        }
        client.webSocket(
            request = {
                url(websocketUrl)
                header(HttpHeaders.Authorization, "Bearer ${settings.token}")
            },
        ) {
            val active = this
            val connectionId = socketGate.withLock { socket = active; ++socketId }
            val replies = Channel<String>(Channel.CONFLATED)
            val heartbeat = launch(Dispatchers.Default) {
                while (isActive) {
                    delay(TauHeartbeatMillis)
                    val request = ListSessions("heartbeat-${newRequestId()}")
                    try {
                        withTimeout(TauHeartbeatMillis * 2) {
                            this@TauClient.send(request, connectionId)
                            while (replies.receive() != request.id) Unit
                        }
                    } catch (_: TimeoutCancellationException) {
                        active.cancel("Tau heartbeat timed out", TauConnectionException("Tau is not responding"))
                        return@launch
                    } catch (error: Throwable) {
                        currentCoroutineContext().ensureActive()
                        active.cancel("Tau connection was lost", error)
                        return@launch
                    }
                }
            }
            try {
                for (frame in incoming) {
                    if (frame !is Frame.Text) continue
                    val frames = mutableListOf(frame)
                    while (frames.size < 64) {
                        val next = incoming.tryReceive().getOrNull() ?: break
                        if (next is Frame.Text) frames.add(next)
                    }
                    val messages = withContext(Dispatchers.Default) {
                        frames.map { TauJson.decodeFromString<ServerMessage>(it.readText()) }
                    }
                    val updates = messages.filterNot { message ->
                        if (message is Response && message.requestId.startsWith("heartbeat-")) {
                            replies.trySend(message.requestId)
                            true
                        } else false
                    }
                    if (updates.isNotEmpty()) onMessages(updates, connectionId)
                }
            } finally {
                heartbeat.cancel()
                replies.close()
                withContext(NonCancellable) {
                    socketGate.withLock {
                        if (socket === active) socket = null
                    }
                }
            }
        }
    }

    suspend fun send(request: ClientRequest, connectionId: Long) = socketGate.withLock {
        val active = socket?.takeIf { socketId == connectionId }
            ?: throw TauConnectionException("Tau is not connected")
        try {
            val frame = withContext(Dispatchers.Default) { Frame.Text(TauJson.encodeToString<ClientRequest>(request)) }
            active.send(frame)
        } catch (error: Throwable) {
            currentCoroutineContext().ensureActive()
            active.cancel("Tau connection was lost", error)
            throw TauConnectionException("Tau connection was lost", error)
        }
    }

    suspend fun downloadAttachment(
        settings: ConnectionSettings,
        sessionId: String,
        entryId: String,
        limit: Long,
        force: Boolean = false,
        allowNetwork: Boolean = true,
        onProgress: suspend (transferred: Long, total: Long?) -> Unit,
    ): String = withContext(Dispatchers.IO) {
        val files = FileSystem.SYSTEM
        val directory = attachmentDirectory().toPath() / settings.identity / sessionId.encodeUtf8().sha256().hex()
        val target = directory / entryId.encodeUtf8().sha256().hex()
        val stored = files.metadataOrNull(target)
        val storedBytes = stored?.size
        if (!force && stored?.isRegularFile == true && storedBytes != null && storedBytes in 0..limit) {
            onProgress(storedBytes, storedBytes)
            return@withContext target.toString()
        }
        if (!allowNetwork) throw AttachmentDownloadException(AttachmentFailure.NotLocal)
        attachmentGate.withPermit {
            files.createDirectories(directory)
            val temporary = directory / ".${target.name}-${newRequestId()}.part"
            val baseUrl = settings.serverUrl.trim().trimEnd('/')
            try {
                client.prepareGet("$baseUrl/v1/sessions/${sessionId.encodeURLPathPart()}/attachments/${entryId.encodeURLPathPart()}") {
                    header(HttpHeaders.Authorization, "Bearer ${settings.token}")
                    timeout {
                        requestTimeoutMillis = HttpTimeoutConfig.INFINITE_TIMEOUT_MS
                        connectTimeoutMillis = 30_000
                        socketTimeoutMillis = 30_000
                    }
                }.execute { response ->
                    if (!response.status.isSuccess()) throw AttachmentDownloadException(AttachmentFailure.Http(response.status.value))
                    val expected = response.headers[HttpHeaders.ContentLength]?.toLongOrNull()
                    if (expected != null && expected !in 0..limit) throw AttachmentDownloadException(AttachmentFailure.TooLarge)
                    val input = response.bodyAsChannel()
                    val buffer = ByteArray(64 * 1024)
                    var transferred = 0L
                    files.openReadWrite(temporary, mustCreate = true).use { handle ->
                        handle.sink().buffer().use { output ->
                            while (true) {
                                val count = input.readAvailable(buffer)
                                if (count < 0) break
                                transferred += count
                                if (transferred > limit) throw AttachmentDownloadException(AttachmentFailure.TooLarge)
                                output.write(buffer, 0, count)
                                onProgress(transferred, expected)
                            }
                            if (expected != null && transferred != expected) throw AttachmentDownloadException(AttachmentFailure.Interrupted)
                            output.flush()
                            handle.flush()
                        }
                    }
                    currentCoroutineContext().ensureActive()
                    files.atomicMove(temporary, target)
                    onProgress(transferred, transferred)
                }
                target.toString()
            } catch (error: Throwable) {
                currentCoroutineContext().ensureActive()
                var cause: Throwable? = error
                while (cause != null) {
                    when (cause) {
                        is AttachmentDownloadException -> throw cause
                        is HttpRequestTimeoutException, is ConnectTimeoutException, is SocketTimeoutException ->
                            throw AttachmentDownloadException(AttachmentFailure.TimedOut, error)
                    }
                    cause = cause.cause
                }
                throw AttachmentDownloadException(AttachmentFailure.Interrupted, error)
            } finally {
                files.delete(temporary, mustExist = false)
            }
        }
    }

    suspend fun uploadFile(
        settings: ConnectionSettings,
        sessionId: String,
        file: PickedFile,
    ): UploadedFile {
        if (file.bytes.isEmpty() || file.bytes.size > MaxUploadBytes) {
            error("${file.name} exceeds Tau's upload limit")
        }
        val baseUrl = settings.serverUrl.trim().trimEnd('/')
        val response = client.post("$baseUrl/v1/sessions/$sessionId/uploads") {
            url { parameters.append("fileName", file.name) }
            header(HttpHeaders.Authorization, "Bearer ${settings.token}")
            contentType(ContentType.Application.OctetStream)
            setBody(file.bytes)
        }
        if (!response.status.isSuccess()) {
            error("File upload failed with HTTP ${response.status.value}")
        }
        return TauJson.decodeFromString(response.body<String>())
    }

    suspend fun uploadPendingCrash(settings: ConnectionSettings) {
        val payload = PlatformServices.pendingCrashReport() ?: return
        val response = client.post("${settings.serverUrl.trim().trimEnd('/')}/v1/telemetry/crash") {
            header(HttpHeaders.Authorization, "Bearer ${settings.token}")
            contentType(ContentType.Application.Json)
            setBody(payload)
        }
        if (response.status.isSuccess()) {
            PlatformServices.clearPendingCrashReport()
        }
    }

    fun close() {
        client.close()
    }
}
