package app.tau

import io.ktor.http.ContentType
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.server.application.Application
import io.ktor.server.application.install
import io.ktor.server.cio.CIO
import io.ktor.server.engine.embeddedServer
import io.ktor.server.response.respond
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
import kotlin.test.assertFailsWith
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
        val stalled = CompletableDeferred<Unit>()
        val imageStatus = AtomicInteger(200)
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            routing {
                get("/v1/sessions/{session}/attachments/{entry}") {
                    assertTrue(call.request.headers[HttpHeaders.Authorization] in listOf("Bearer first", "Bearer second"))
                    reads.incrementAndGet()
                    when (call.parameters["entry"]) {
                        "redirect" -> call.respondRedirect("/forbidden")
                        "unauthorized" -> call.respond(HttpStatusCode.Unauthorized)
                        "missing" -> call.respond(HttpStatusCode.NotFound)
                        "image" -> call.respondBytes(png, ContentType.Image.PNG, HttpStatusCode.fromValue(imageStatus.get()))
                        "timeout" -> call.respondBytesWriter(ContentType.Image.PNG) {
                            writeFully(png.take(16).toByteArray()); flush()
                            stalled.await()
                        }
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
            assertEquals(AttachmentFailure.NotLocal, assertFailsWith<AttachmentDownloadException> {
                client.downloadAttachment(settings, "chat", "missing", 128, allowNetwork = false) { _, _ -> }
            }.failure)
            for ((entry, failure) in listOf(
                "large" to AttachmentFailure.TooLarge, "chunked" to AttachmentFailure.TooLarge,
                "redirect" to AttachmentFailure.Http(302), "unauthorized" to AttachmentFailure.Http(401),
                "missing" to AttachmentFailure.Http(404),
            )) {
                assertEquals(failure, assertFailsWith<AttachmentDownloadException> {
                    client.downloadAttachment(settings, "chat", entry, 128) { _, _ -> }
                }.failure)
            }
            withTimeout(40_000) {
                val timeout = assertFailsWith<AttachmentDownloadException> {
                    client.downloadAttachment(settings, "chat", "timeout", 128) { _, _ -> }
                }
                assertEquals(AttachmentFailure.TimedOut, timeout.failure, timeout.stackTraceToString())
            }
            stalled.complete(Unit)
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
                assertEquals(AttachmentFailure.Interrupted, assertFailsWith<AttachmentDownloadException> {
                    client.downloadAttachment(settings.copy(serverUrl = "http://127.0.0.1:${listener.localPort}"),
                        "chat", "truncated", 1024) { _, _ -> }
                }.failure)
                peer.await()
            }
            assertTrue(root.walk().none { it.name.endsWith(".part") })
            assertEquals(2, root.walk().count { it.isFile })
            val before = reads.get()
            assertEquals(path, client.downloadAttachment(settings, "chat", "image", 10_000_000, force = true) { _, _ -> })
            assertEquals(before + 1, reads.get())
            assertContentEquals(png, java.io.File(path).readBytes())
            imageStatus.set(502)
            assertEquals(AttachmentFailure.Http(502), assertFailsWith<AttachmentDownloadException> {
                client.downloadAttachment(settings, "chat", "image", 10_000_000, force = true) { _, _ -> }
            }.failure)
            assertEquals(path, client.downloadAttachment(settings, "chat", "image", 10_000_000, allowNetwork = false) { _, _ -> })
            assertContentEquals(png, java.io.File(path).readBytes())
            assertTrue(root.walk().none { it.name.endsWith(".part") })
        } finally { stalled.complete(Unit); release.complete(Unit); client.close(); server.stop(0, 1000); root.deleteRecursively() }
    }

    @Test
    fun controllerKeepsTransfersAndHandlesPreviewReloadSaveAndOfflineRetry(): Unit = runBlocking {
        val root = Files.createTempDirectory("tau-image-controller").toFile()
        val path = root.resolve("transcript.db").path
        val home = System.getProperty("user.home")
        val started = CompletableDeferred<Unit>()
        val release = CompletableDeferred<Unit>()
        val cancelStarted = CompletableDeferred<Unit>()
        val cancelRelease = CompletableDeferred<Unit>()
        val reads = AtomicInteger()
        val chat = SessionSummary("image-chat", "Images", SessionStatus.Idle, createdAtMs = 1, updatedAtMs = 1)
        val other = chat.copy(id = "other-chat")
        val image = TranscriptEntry("image", role = EntryRole.Tool, attachment = ChatAttachment(AttachmentKind.Image, "image.png", size = png.size.toLong()))
        val module: Application.() -> Unit = {
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
                    if (call.parameters["entry"] == "cancel") {
                        cancelStarted.complete(Unit)
                        cancelRelease.await()
                    }
                    release.await()
                    if (call.parameters["entry"] == "failed") call.respond(HttpStatusCode.NotFound)
                    else call.respondBytes(png, ContentType.Image.PNG)
                }
            }
        }
        var server = embeddedServer(CIO, host = "127.0.0.1", port = 0, module = module).start(wait = false)
        val port = server.engine.resolvedConnectors().single().port
        val settings = ConnectionSettings("http://127.0.0.1:$port", "local-test")
        var controller = TauController(Dispatchers.Swing, TranscriptStore({ path }))
        try {
            withContext(Dispatchers.Swing) { controller.start(settings) }
            withTimeout(10_000) {
                while (controller.state.value.connectionStatus != ConnectionStatus.Connected || controller.state.value.selectedSessionId != chat.id) delay(10)
            }
            withContext(Dispatchers.Swing) {
                controller.downloadAttachment(chat.id, image, AttachmentDownloadAction.Preview)
                controller.downloadAttachment(chat.id, image, AttachmentDownloadAction.Preview)
                controller.downloadAttachment(chat.id, image, AttachmentDownloadAction.Reload)
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
                controller.downloadAttachment(chat.id, image, AttachmentDownloadAction.Preview)
            }
            withTimeout(10_000) { while (controller.state.value.attachmentDownloads[key]?.localPath == null) delay(10) }
            assertEquals(1, reads.get())
            assertEquals(saved, controller.state.value.attachmentDownloads[key]?.localPath)
            val missing = image.copy(id = "not-local")
            val missingKey = AttachmentDownloadKey(chat.id, missing.id)
            withContext(Dispatchers.Swing) { controller.downloadAttachment(chat.id, missing, AttachmentDownloadAction.Preview) }
            withTimeout(10_000) { while (controller.state.value.attachmentDownloads[missingKey]?.failure == null) delay(10) }
            assertEquals(AttachmentFailure.NotLocal, controller.state.value.attachmentDownloads[missingKey]?.failure)
            withContext(Dispatchers.Swing) { controller.downloadAttachment(chat.id, missing, AttachmentDownloadAction.Preview) }
            assertEquals(1, controller.state.value.attachmentDownloads[missingKey]?.attempt)
            assertEquals(1, reads.get())

            server = embeddedServer(CIO, host = "127.0.0.1", port = port, module = module).start(wait = false)
            withTimeout(10_000) { while (controller.state.value.connectionStatus != ConnectionStatus.Connected) delay(10) }
            withContext(Dispatchers.Swing) {
                controller.downloadAttachment(chat.id, image, AttachmentDownloadAction.Preview)
                controller.downloadAttachment(chat.id, missing, AttachmentDownloadAction.Preview)
                controller.downloadAttachment(chat.id, missing, AttachmentDownloadAction.Preview)
            }
            withTimeout(10_000) { while (controller.state.value.attachmentDownloads[missingKey]?.status != AttachmentDownloadStatus.Downloaded) delay(10) }
            assertEquals(2, reads.get())
            assertEquals(1, controller.state.value.attachmentDownloads[key]?.attempt)
            assertEquals(2, controller.state.value.attachmentDownloads[missingKey]?.attempt)
            withContext(Dispatchers.Swing) { controller.downloadAttachment(chat.id, image, AttachmentDownloadAction.Reload) }
            withTimeout(10_000) { while (controller.state.value.attachmentDownloads[key]?.status != AttachmentDownloadStatus.Downloaded) delay(10) }
            assertEquals(3, reads.get())
            assertEquals(2, controller.state.value.attachmentDownloads[key]?.attempt)

            val failed = image.copy(id = "failed")
            val failedKey = AttachmentDownloadKey(chat.id, failed.id)
            withContext(Dispatchers.Swing) { controller.downloadAttachment(chat.id, failed, AttachmentDownloadAction.Preview) }
            withTimeout(10_000) { while (controller.state.value.attachmentDownloads[failedKey]?.failure == null) delay(10) }
            assertEquals(AttachmentFailure.Http(404), controller.state.value.attachmentDownloads[failedKey]?.failure)
            withContext(Dispatchers.Swing) { controller.downloadAttachment(chat.id, failed, AttachmentDownloadAction.Preview) }
            assertEquals(1, controller.state.value.attachmentDownloads[failedKey]?.attempt)
            assertEquals(4, reads.get())

            val cancelled = image.copy(id = "cancel")
            val cancelledKey = AttachmentDownloadKey(chat.id, cancelled.id)
            withContext(Dispatchers.Swing) { controller.downloadAttachment(chat.id, cancelled, AttachmentDownloadAction.Preview) }
            withTimeout(10_000) { cancelStarted.await() }
            withContext(Dispatchers.Swing) {
                controller.cancelAttachmentDownload(cancelled)
                controller.downloadAttachment(chat.id, cancelled, AttachmentDownloadAction.Preview)
            }
            cancelRelease.complete(Unit)
            assertEquals(AttachmentFailure.Cancelled, controller.state.value.attachmentDownloads[cancelledKey]?.failure)
            assertEquals(1, controller.state.value.attachmentDownloads[cancelledKey]?.attempt)
            assertEquals(5, reads.get())

            server.stop(0, 1000)
            withTimeout(10_000) { while (controller.state.value.connectionStatus == ConnectionStatus.Connected) delay(10) }
            System.setProperty("user.home", root.path)
            withContext(Dispatchers.Swing) { controller.downloadAttachment(chat.id, image, AttachmentDownloadAction.Save) }
            withTimeout(10_000) { while (controller.state.value.attachmentDownloads[key]?.saved == null) delay(10) }
            val exported = checkNotNull(controller.state.value.attachmentDownloads[key]?.saved)
            assertTrue(java.io.File(exported.reference).toPath().startsWith(root.toPath()))
            assertContentEquals(png, java.io.File(exported.reference).readBytes())
            assertEquals("Saved to ${exported.location}", controller.state.value.notice)
            assertEquals(saved, controller.state.value.attachmentDownloads[key]?.localPath)
            assertEquals(5, reads.get())
            java.io.File(saved).parentFile.deleteRecursively()
        } finally {
            cancelRelease.complete(Unit); release.complete(Unit)
            withContext(Dispatchers.Swing) { controller.dispose() }.join()
            System.setProperty("user.home", home)
            server.stop(0, 1000); root.deleteRecursively()
        }
    }
}
