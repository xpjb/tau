package app.tau

import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.plugins.websocket.DefaultClientWebSocketSession
import io.ktor.client.plugins.websocket.WebSockets
import io.ktor.client.plugins.websocket.webSocket
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.request.post
import io.ktor.client.request.setBody
import io.ktor.client.request.url
import io.ktor.http.ContentType
import io.ktor.http.HttpHeaders
import io.ktor.http.contentType
import io.ktor.http.isSuccess
import io.ktor.websocket.Frame
import io.ktor.websocket.readText
import io.ktor.websocket.send
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.encodeToString

class TauClient {
    private val client = HttpClient(platformHttpEngine()) {
        install(WebSockets)
    }
    private val socketGate = Mutex()
    private var socket: DefaultClientWebSocketSession? = null

    suspend fun run(settings: ConnectionSettings, onMessage: suspend (ServerMessage) -> Unit) {
        val baseUrl = settings.serverUrl.trim().trimEnd('/')
        require(baseUrl.startsWith("http://") || baseUrl.startsWith("https://")) {
            "Server URL must start with http:// or https://"
        }
        val websocketUrl = when {
            baseUrl.startsWith("https://") -> "wss://${baseUrl.removePrefix("https://")}/v1/ws"
            else -> "ws://${baseUrl.removePrefix("http://")}/v1/ws"
        }
        client.webSocket(
            request = {
                url(websocketUrl)
                header(HttpHeaders.Authorization, "Bearer ${settings.token}")
            },
        ) {
            socketGate.withLock { socket = this }
            try {
                for (frame in incoming) {
                    if (frame is Frame.Text) {
                        onMessage(TauJson.decodeFromString<ServerMessage>(frame.readText()))
                    }
                }
            } finally {
                socketGate.withLock {
                    if (socket === this) socket = null
                }
            }
        }
    }

    suspend fun send(request: ClientRequest) {
        val active = socketGate.withLock { socket }
            ?: error("Tau is not connected")
        active.send(Frame.Text(TauJson.encodeToString<ClientRequest>(request)))
    }

    suspend fun downloadAttachment(
        settings: ConnectionSettings,
        sessionId: String,
        entryId: String,
        fileName: String,
    ): String {
        val baseUrl = settings.serverUrl.trim().trimEnd('/')
        val response = client.get("$baseUrl/v1/sessions/$sessionId/attachments/$entryId") {
            header(HttpHeaders.Authorization, "Bearer ${settings.token}")
        }
        if (!response.status.isSuccess()) {
            error("Attachment download failed with HTTP ${response.status.value}")
        }
        val bytes = response.body<ByteArray>()
        if (bytes.size > 50_000_000) error("Attachment exceeds Tau's download limit")
        return PlatformServices.saveDownload(fileName, bytes)
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
