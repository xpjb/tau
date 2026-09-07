package app.tau

import io.ktor.http.ContentType
import io.ktor.http.HttpHeaders
import io.ktor.server.application.install
import io.ktor.server.cio.CIO
import io.ktor.server.engine.embeddedServer
import io.ktor.server.response.respondBytes
import io.ktor.server.response.respondBytesWriter
import io.ktor.server.response.respondRedirect
import io.ktor.server.routing.get
import io.ktor.server.routing.routing
import io.ktor.server.websocket.WebSockets
import io.ktor.server.websocket.webSocket
import io.ktor.utils.io.writeFully
import io.ktor.websocket.Frame
import io.ktor.websocket.readText
import io.ktor.websocket.send
import java.nio.file.Files
import java.net.ServerSocket
import java.util.Base64
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.swing.Swing
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFails
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

class AttachmentFileTest {
    private val png = Base64.getDecoder().decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/l9sAAAAASUVORK5CYII=")

    @Test
    fun cachesCompleteFilesRejectsBadTransfersAndRestoresOffline(): Unit = runBlocking {
        val root = Files.createTempDirectory("tau-attachment-files").toFile()
        val reads = AtomicInteger()
        val started = CompletableDeferred<Unit>()
        val release = CompletableDeferred<Unit>()
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            routing {
                get("/v1/sessions/{session}/attachments/{entry}") {
                    assertTrue(call.request.headers[HttpHeaders.Authorization] in listOf("Bearer first", "Bearer second"))
                    reads.incrementAndGet()
                    when (call.parameters["entry"]) {
                        "redirect" -> call.respondRedirect("/forbidden")
                        "large" -> call.respondBytes(ByteArray(129), ContentType.Application.OctetStream)
                        "chunked" -> call.respondBytesWriter { writeFully(ByteArray(129)) }
                        "cancel" -> call.respondBytesWriter(ContentType.Image.PNG) {
                            writeFully(png.take(16).toByteArray()); flush()
                            started.complete(Unit)
                            release.await()
                            writeFully(png.drop(16).toByteArray())
                        }
                        else -> call.respondBytes(png, ContentType.Image.PNG)
                    }
                }
                get("/forbidden") { error("Attachment request followed a redirect") }
            }
        }.start(wait = false)
        val port = server.engine.resolvedConnectors().single().port
        val settings = ConnectionSettings("http://127.0.0.1:$port", "first")
        var client = TauClient { root.path }
        try {
            val path = client.downloadAttachment(settings, "chat", "image", 10_000_000) { _, _ -> }
            assertContentEquals(png, java.io.File(path).readBytes())
            assertEquals(1, reads.get())
            client.close(); client = TauClient { root.path }
            assertEquals(path, client.downloadAttachment(settings, "chat", "image", 10_000_000, allowNetwork = false) { _, _ -> })
            assertEquals(1, reads.get())
            val other = client.downloadAttachment(settings.copy(token = "second"), "chat", "image", 10_000_000) { _, _ -> }
            assertNotEquals(path, other)
            assertEquals(2, reads.get())
            assertFails { client.downloadAttachment(settings, "chat", "missing", 128, allowNetwork = false) { _, _ -> } }
            for (entry in listOf("large", "chunked", "redirect")) {
                assertFails { client.downloadAttachment(settings, "chat", entry, 128) { _, _ -> } }
            }
            val cancelled = async { client.downloadAttachment(settings, "chat", "cancel", 10_000_000) { _, _ -> } }
            withTimeout(10_000) { started.await() }
            cancelled.cancelAndJoin()
            release.complete(Unit)
            assertTrue(root.walk().none { it.name.endsWith(".part") })
            ServerSocket(0, 1, java.net.InetAddress.getLoopbackAddress()).use { listener ->
                listener.soTimeout = 10_000
                val peer = async(Dispatchers.IO) {
                    listener.accept().use { socket ->
                        socket.soTimeout = 10_000
                        val input = socket.getInputStream().bufferedReader()
                        while (!input.readLine().isNullOrEmpty()) {}
                        socket.getOutputStream().write("HTTP/1.1 200 OK\r\nContent-Length: 256\r\nConnection: close\r\n\r\nshort".toByteArray())
                        socket.getOutputStream().flush()
                    }
                }
                assertFails { client.downloadAttachment(settings.copy(serverUrl = "http://127.0.0.1:${listener.localPort}"),
                    "chat", "truncated", 1024) { _, _ -> } }
                peer.await()
            }
            assertTrue(root.walk().none { it.name.endsWith(".part") })
            assertEquals(2, root.walk().count { it.isFile })
            val before = reads.get()
            assertEquals(path, client.downloadAttachment(settings, "chat", "image", 10_000_000, force = true) { _, _ -> })
            assertEquals(before + 1, reads.get())
            assertContentEquals(png, java.io.File(path).readBytes())
        } finally { release.complete(Unit); client.close(); server.stop(0, 1000); root.deleteRecursively() }
    }

    @Test
    fun controllerKeepsOneImageTransferAcrossSelectionAndTheOldTimeout(): Unit = runBlocking {
        val root = Files.createTempDirectory("tau-image-controller").toFile()
        val path = root.resolve("transcript.db").path
        val started = CompletableDeferred<Unit>()
        val release = CompletableDeferred<Unit>()
        val reads = AtomicInteger()
        val chat = SessionSummary("image-chat", "Images", SessionStatus.Idle, createdAtMs = 1, updatedAtMs = 1)
        val other = chat.copy(id = "other-chat")
        val image = TranscriptEntry("image", role = EntryRole.Tool, attachment = ChatAttachment(AttachmentKind.Image, "image.png", size = png.size.toLong()))
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                webSocket("/v1/ws") {
                    send(TauJson.encodeToString<ServerMessage>(Hello(TauProtocolVersion, "test")))
                    send(TauJson.encodeToString<ServerMessage>(Sessions(listOf(chat, other))))
                    for (frame in incoming) if (frame is Frame.Text) {
                        val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                        if (request is OpenSession) {
                            val entries = if (request.sessionId == chat.id) listOf(image) else emptyList()
                            send(TauJson.encodeToString<ServerMessage>(TranscriptSnapshot(request.sessionId,
                                TranscriptCut("g", 0, entries.lastOrNull()?.id, entries, QueueState()))))
                        }
                        assertTrue(request is OpenSession || request is ListSessions)
                        send(TauJson.encodeToString<ServerMessage>(Response(request.id, true)))
                    }
                }
                get("/v1/sessions/{session}/attachments/{entry}") {
                    assertEquals(chat.id, call.parameters["session"])
                    reads.incrementAndGet(); started.complete(Unit)
                    release.await()
                    call.respondBytes(png, ContentType.Image.PNG)
                }
            }
        }.start(wait = false)
        val port = server.engine.resolvedConnectors().single().port
        val settings = ConnectionSettings("http://127.0.0.1:$port", "local-test")
        var controller = TauController(Dispatchers.Swing, TranscriptStore({ path }))
        try {
            withContext(Dispatchers.Swing) { controller.start(settings) }
            withTimeout(10_000) {
                while (controller.state.value.connectionStatus != ConnectionStatus.Connected || controller.state.value.selectedSessionId != chat.id) delay(10)
            }
            withContext(Dispatchers.Swing) {
                controller.downloadAttachment(chat.id, image, save = false, automatic = true)
                controller.downloadAttachment(chat.id, image, save = false, automatic = true)
                controller.downloadAttachment(chat.id, image, save = false)
            }
            withTimeout(10_000) { started.await() }
            withContext(Dispatchers.Swing) { controller.selectSession(other.id) }
            delay(16_000)
            assertEquals(1, reads.get())
            val key = AttachmentDownloadKey(chat.id, image.id)
            assertEquals(AttachmentDownloadStatus.Downloading, controller.state.value.attachmentDownloads[key]?.status)
            release.complete(Unit)
            val saved = withTimeout(10_000) {
                while (controller.state.value.attachmentDownloads[key]?.localPath == null) delay(10)
                checkNotNull(controller.state.value.attachmentDownloads[key]?.localPath)
            }
            assertContentEquals(png, java.io.File(saved).readBytes())
            withContext(Dispatchers.Swing) { controller.dispose() }.join()
            server.stop(0, 1000)
            controller = TauController(Dispatchers.Swing, TranscriptStore({ path }))
            withContext(Dispatchers.Swing) { controller.start(settings) }
            withTimeout(10_000) { while (controller.state.value.restoring) delay(10) }
            withContext(Dispatchers.Swing) {
                controller.selectSession(chat.id)
                controller.downloadAttachment(chat.id, image, save = false, automatic = true)
            }
            withTimeout(10_000) { while (controller.state.value.attachmentDownloads[key]?.localPath == null) delay(10) }
            assertEquals(1, reads.get())
            assertEquals(saved, controller.state.value.attachmentDownloads[key]?.localPath)
            java.io.File(saved).parentFile.deleteRecursively()
        } finally { release.complete(Unit); withContext(Dispatchers.Swing) { controller.dispose() }.join(); server.stop(0, 1000); root.deleteRecursively() }
    }
}
