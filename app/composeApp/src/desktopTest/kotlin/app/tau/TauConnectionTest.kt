package app.tau

import androidx.compose.runtime.snapshots.Snapshot
import io.ktor.http.HttpHeaders
import io.ktor.server.cio.CIO
import io.ktor.server.engine.embeddedServer
import io.ktor.server.application.install
import io.ktor.server.routing.routing
import io.ktor.server.websocket.DefaultWebSocketServerSession
import io.ktor.server.websocket.WebSockets
import io.ktor.server.websocket.webSocket
import io.ktor.server.websocket.webSocketRaw
import io.ktor.websocket.CloseReason
import io.ktor.websocket.Frame
import io.ktor.websocket.WebSocketSession
import io.ktor.websocket.close
import io.ktor.websocket.readText
import io.ktor.websocket.send
import java.nio.file.Files
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import kotlin.time.Duration
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.swing.Swing
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertIs
import kotlin.test.assertNull
import kotlin.test.assertNotEquals
import kotlin.test.assertSame
import kotlin.test.assertTrue

class TauConnectionTest {
    private val chat = SessionSummary("chat", "Test chat", SessionStatus.Idle, createdAtMs = 1, updatedAtMs = 1,
        contextUsage = ContextUsage(64000, 200000))
    private val queue = QueueState(available = true, runId = "run",
        capabilities = listOf("queue_edit", "queue_delete", "queue_run_prefix", "queue_resume", "queue_cancel_control"),
        boundaries = listOf("reasoning_checkpoint", "turn"))

    private val TauUiState.unread: Set<String>
        get() = sessions.filter { isUnread(it) }.mapTo(mutableSetOf()) { it.id }

    private suspend fun WebSocketSession.sendMessage(message: ServerMessage) {
        send(TauJson.encodeToString<ServerMessage>(message))
    }

    private suspend fun TauController.awaitState(timeout: Long = 10_000, predicate: (TauUiState) -> Boolean): TauUiState = withTimeout(timeout) {
        while (true) {
            val state = state.value
            val read = Snapshot.takeSnapshot()
            val matches = try { read.enter { predicate(state) } } finally { read.dispose() }
            if (matches) return@withTimeout state
            delay(10)
        }
        error("unreachable")
    }

    private suspend fun Channel<ClientRequest>.nextRequest(): ClientRequest = withTimeout(10_000) { receive() }


    @Test
    fun edits_title_prompt_and_reloads_after_disconnect_without_replaying_saves() = runBlocking<Unit> {
        val root = Files.createTempDirectory("tau-title-client").toFile()
        val sockets = Channel<DefaultWebSocketServerSession>(Channel.UNLIMITED)
        val requests = Channel<ClientRequest>(Channel.UNLIMITED)
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                webSocket("/v1/ws") {
                    sendMessage(Hello(TauProtocolVersion, "fixture"))
                    sendMessage(Sessions(emptyList()))
                    sockets.send(this)
                    for (frame in incoming) if (frame is Frame.Text) {
                        val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                        if (request is ListSessions && request.id.startsWith("heartbeat-")) sendMessage(Response(request.id, true))
                        else requests.send(request)
                    }
                }
            }
        }.start(wait = false)
        val settings = ConnectionSettings("http://127.0.0.1:${server.engine.resolvedConnectors().single().port}", "fixture")
        var controller = TauController(Dispatchers.Swing, LocalStore({ root.resolve("local.db").path }))
        val default = "Default rules\n{text}\nTitle:"
        val custom = "  Full \"system\" prompt 🔧\n{text}\n  "
        try {
            withContext(Dispatchers.Swing) { controller.start(settings) }
            var socket = withTimeout(10_000) { sockets.receive() }
            controller.awaitState { it.connectionStatus == ConnectionStatus.Connected }
            assertNull(controller.state.value.titlePrompt)
            withContext(Dispatchers.Swing) { controller.showSettings() }
            val read = assertIs<GetTitlePrompt>(requests.nextRequest())
            socket.sendMessage(TitlePrompt(read.id, default, default))
            socket.sendMessage(Response(read.id, true))
            controller.awaitState { it.titlePrompt?.prompt == default && !it.titlePromptPending }
            withContext(Dispatchers.Swing) { controller.titlePrompt() }
            val quietRead = assertIs<GetTitlePrompt>(requests.nextRequest())
            assertNull(controller.awaitState(15_000) { !it.titlePromptPending }.error, "A title-settings read produced a timeout banner")
            socket.sendMessage(TitlePrompt(quietRead.id, "Stale read", default))
            socket.sendMessage(Response(quietRead.id, true))
            delay(50)
            assertEquals(default, controller.state.value.titlePrompt?.prompt)
            withContext(Dispatchers.Swing) { controller.titlePrompt(custom); controller.titlePrompt(custom) }
            val rejected = assertIs<SetTitlePrompt>(requests.nextRequest())
            assertEquals(custom, rejected.prompt)
            socket.sendMessage(Response(rejected.id, false, error = "Disk full"))
            controller.awaitState { it.error == "Disk full" && !it.titlePromptPending }
            assertEquals(default, controller.state.value.titlePrompt?.prompt)
            assertTrue(requests.tryReceive().isFailure)
            for (value in listOf(custom, "")) {
                withContext(Dispatchers.Swing) { controller.titlePrompt(value) }
                val saved = assertIs<SetTitlePrompt>(requests.nextRequest())
                assertEquals(value, saved.prompt)
                socket.sendMessage(TitlePrompt(saved.id, value, default))
                socket.sendMessage(Response(saved.id, true))
                controller.awaitState { it.titlePrompt?.prompt == value && !it.titlePromptPending }
            }
            withContext(Dispatchers.Swing) { controller.titlePrompt("Unconfirmed") }
            val timedOut = assertIs<SetTitlePrompt>(requests.nextRequest())
            controller.awaitState(15_000) { !it.titlePromptPending && it.error == "Title prompt request timed out" }
            socket.sendMessage(TitlePrompt(timedOut.id, "Stale", default))
            socket.sendMessage(Response(timedOut.id, true))
            delay(50)
            assertEquals("", controller.state.value.titlePrompt?.prompt)
            withContext(Dispatchers.Swing) { controller.titlePrompt(custom) }
            val lost = assertIs<SetTitlePrompt>(requests.nextRequest())
            socket.close(CloseReason(CloseReason.Codes.GOING_AWAY, "save reply lost"))
            socket = withTimeout(10_000) { sockets.receive() }
            val refreshed = assertIs<GetTitlePrompt>(requests.nextRequest())
            assertNotEquals(lost.id, refreshed.id)
            socket.sendMessage(TitlePrompt(lost.id, "Stale", default))
            socket.sendMessage(TitlePrompt(refreshed.id, custom, default))
            socket.sendMessage(Response(refreshed.id, true))
            controller.awaitState { it.titlePrompt?.prompt == custom && !it.titlePromptPending }
            assertTrue(requests.tryReceive().isFailure, "Reconnect replayed a save")
            withContext(Dispatchers.Swing) { controller.dispose() }.join()
            controller = TauController(Dispatchers.Swing, LocalStore({ root.resolve("local.db").path }))
            withContext(Dispatchers.Swing) { controller.start(settings) }
            socket = withTimeout(10_000) { sockets.receive() }
            controller.awaitState { it.connectionStatus == ConnectionStatus.Connected }
            assertNull(controller.state.value.titlePrompt)
            withContext(Dispatchers.Swing) { controller.showSettings() }
            val reopened = assertIs<GetTitlePrompt>(requests.nextRequest())
            socket.sendMessage(TitlePrompt(reopened.id, custom, default))
            socket.sendMessage(Response(reopened.id, true))
            controller.awaitState { it.titlePrompt?.prompt == custom && !it.titlePromptPending }
        } finally {
            withContext(Dispatchers.Swing) { controller.dispose() }.join()
            server.stop(0, 1000)
            root.deleteRecursively()
        }
    }

    @Test
    fun surfaces_codex_usage_notices_as_quota_without_dialog() = runBlocking {
        val directory = Files.createTempDirectory("tau-usage")
        val path = directory.resolve("transcript.db").toString()
        val codexChat = chat.copy(model = SessionModel("openai-codex", "gpt-5.6-sol"))
        val sockets = Channel<DefaultWebSocketServerSession>(Channel.UNLIMITED)
        val requests = Channel<ClientRequest>(Channel.UNLIMITED)
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                webSocket("/v1/ws") {
                    sendMessage(Hello(TauProtocolVersion, "test"))
                    sendMessage(Sessions(listOf(codexChat)))
                    sockets.send(this)
                    for (frame in incoming) if (frame is Frame.Text) {
                        val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                        when (request) {
                            is ListSessions -> if (request.id.startsWith("heartbeat-")) sendMessage(Response(request.id, true)) else requests.send(request)
                            is Prompt -> if (request.text.startsWith("/vibe-bridge-usage ")) {
                                val digits = request.text.substringAfterLast(' ')
                                sendMessage(Response(request.id, true, chat.id))
                                sendMessage(ExtensionUi(chat.id, ExtensionUiRequest(id = "usage-notice", method = "notify",
                                    message = "VIBE_BRIDGE_CODEX_USAGE:" + """{"version":2,"requestId":$digits,""" +
                                        """"text":"Codex quota (pro)","report":{"provider":"openai-codex","fetchedAtMs":1788856497357,""" +
                                        """"plan":"pro","limitReached":false,"windows":[{"id":"primary_window","label":"Weekly",""" +
                                        """"durationSeconds":604800,"remainingPercent":96.0,"resetsAtMs":${System.currentTimeMillis() + 600_000}}]}}""")))
                            }
                            else -> requests.send(request)
                        }
                    }
                }
            }
        }.start(wait = false)
        val port = server.engine.resolvedConnectors().single().port
        val settings = ConnectionSettings("http://127.0.0.1:$port", "test-token")
        var controller = TauController(Dispatchers.Swing, LocalStore({ path }))
        try {
            withContext(Dispatchers.Swing) { controller.start(settings) }
            val socket = withTimeout(10_000) { sockets.receive() }
            val open = assertIs<OpenSession>(requests.nextRequest())
            val user = TranscriptEvent("entry:u0:0", 0, "u0", role = EventRole.User, kind = EventKind.Text, text = "Start")
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 0, listOf(user), queue)))
            socket.sendMessage(Response(open.id, true, chat.id))
            controller.awaitState { it.transcripts[chat.id]?.synchronized == true }
            controller.refreshUsage(chat.id, force = true)
            controller.awaitState { it.codexUsage != null }
            val usage = controller.state.value.codexUsage!!
            assertEquals("openai-codex", usage.provider)
            assertEquals("pro", usage.plan)
            assertEquals("Weekly", usage.windows.single().label)
            assertEquals(96.0, usage.windows.single().remainingPercent)
            assertTrue(usage.windows.single().resetsAtMs!! > System.currentTimeMillis())
            assertTrue(controller.state.value.extensionDialogs.isEmpty())
            assertNull(controller.state.value.notice)
        } finally {
            controller.dispose()
            server.stop(1_000, 1_000)
            directory.toFile().deleteRecursively()
        }
    }


    @Test
    fun derives_unread_and_model_failure_status() = runBlocking {
        val directory = Files.createTempDirectory("tau-unread")
        val path = directory.resolve("transcript.db").toString()
        val quiet = chat.copy(id = "quiet", title = "Quiet")
        val catalog = AtomicReference(listOf(chat, quiet))
        val sockets = Channel<DefaultWebSocketServerSession>(Channel.UNLIMITED)
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                webSocket("/v1/ws") {
                    sendMessage(Hello(TauProtocolVersion, "test"))
                    sendMessage(Sessions(catalog.get()))
                    sockets.send(this)
                    for (frame in incoming) if (frame is Frame.Text) {
                        val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                        when (request) {
                            is ListSessions -> if (request.id.startsWith("heartbeat-")) sendMessage(Response(request.id, true))
                            is OpenSession -> {
                                sendMessage(TranscriptSnapshot(request.sessionId, TranscriptCut("g", 0, emptyList(), queue)))
                                sendMessage(Response(request.id, true, request.sessionId))
                            }
                            else -> {}
                        }
                    }
                }
            }
        }.start(wait = false)
        val port = server.engine.resolvedConnectors().single().port
        val settings = ConnectionSettings("http://127.0.0.1:$port", "test-token")
        var controller = TauController(Dispatchers.Swing, LocalStore({ path }))
        try {
            withContext(Dispatchers.Swing) { controller.start(settings) }
            val socket = withTimeout(10_000) { sockets.receive() }
            controller.awaitState { !it.restoring && it.sessions.size == 2 }
            assertTrue(controller.state.value.unread.isEmpty())
            catalog.set(listOf(quiet.copy(updatedAtMs = 2, status = SessionStatus.Starting), chat))
            socket.sendMessage(Sessions(catalog.get()))
            controller.awaitState { it.sessions.first().id == quiet.id && it.sessions.first().status == SessionStatus.Starting }
            assertTrue(quiet.id !in controller.state.value.unread, "Starting must take priority over unread")
            for (status in listOf(SessionStatus.Running, SessionStatus.Idle, SessionStatus.Starting,
                SessionStatus.Running, SessionStatus.Sleeping, SessionStatus.Error, SessionStatus.Idle)) {
                socket.sendMessage(SessionState(quiet.id, status))
                controller.awaitState { it.sessions.first().status == status }
                assertEquals(status != SessionStatus.Starting && status != SessionStatus.Running,
                    quiet.id in controller.state.value.unread, "Unread eligibility for $status")
                assertEquals(1L, controller.state.value.readAt[quiet.id], "Active status never consumes unseen activity")
            }
            withContext(Dispatchers.Swing) { controller.selectSession("quiet") }
            assertTrue(controller.state.value.unread.isEmpty())
            catalog.set(listOf(chat.copy(updatedAtMs = 3), quiet.copy(updatedAtMs = 2)))
            socket.sendMessage(Sessions(catalog.get()))
            controller.awaitState { chat.id in it.unread }
            withContext(Dispatchers.Swing) { controller.selectSession(chat.id) }
            catalog.set(listOf(quiet.copy(updatedAtMs = 4, status = SessionStatus.Running), chat.copy(updatedAtMs = 3)))
            socket.sendMessage(Sessions(catalog.get()))
            controller.awaitState { it.sessions.first().id == quiet.id && it.sessions.first().updatedAtMs == 4L }
            assertTrue(quiet.id !in controller.state.value.unread)
            withContext(Dispatchers.Swing) { controller.dispose() }.join()
            controller = TauController(Dispatchers.Swing, LocalStore({ path }))
            withContext(Dispatchers.Swing) { controller.start(settings) }
            val reopened = withTimeout(10_000) { sockets.receive() }
            controller.awaitState { it.transcripts[quiet.id]?.synchronized == true && it.sessions.first().status == SessionStatus.Running }
            assertTrue(quiet.id !in controller.state.value.unread, "Restored running chats stay active, not unread")
            assertEquals(2L, controller.state.value.readAt[quiet.id], "Reading metadata and warming history preserve the read marker")
            reopened.sendMessage(SessionState(quiet.id, SessionStatus.Idle))
            controller.awaitState { quiet.id in it.unread }
            assertEquals(4L, controller.state.value.sessions.first().updatedAtMs, "Completion needs no extra activity to reveal unread")

            val call = TranscriptEvent("call", 0, "call", role = EventRole.Assistant, kind = EventKind.Tool,
                toolName = "bash", toolCallId = "bash", text = "exit 1", stopReason = "toolUse")
            val tool = TranscriptEvent("tool", 1, "tool", role = EventRole.Tool, kind = EventKind.Text,
                toolName = "bash", toolCallId = "bash", text = "Exit code 1", isError = true)
            val reply = TranscriptEvent("reply", 2, "reply", role = EventRole.Assistant, kind = EventKind.Hidden,
                stopReason = "error", errorMessage = "Provider failed")
            val system = TranscriptEvent("system", 3, "system", role = EventRole.System, kind = EventKind.Text, text = "Notice")
            val user = TranscriptEvent("user", 4, "user", role = EventRole.User, kind = EventKind.Text, text = "Try again")
            val settled = TranscriptEvent("settled", 5, "settled", role = EventRole.Assistant, kind = EventKind.Text,
                text = "Recovered", stopReason = "stop")
            val cases = listOf(
                listOf(call, tool.copy(phase = EventPhase.Live)) to false,
                listOf(call, tool) to false,
                listOf(call, tool, reply.copy(phase = EventPhase.Live)) to false,
                listOf(call, tool, reply, system) to true,
                listOf(reply, system, tool.copy(order = 4)) to true,
                listOf(reply, system, user) to false,
                listOf(reply, user, settled.copy(isError = true)) to false,
                listOf(reply, user, settled.copy(stopReason = "aborted")) to false,
                listOf(settled.copy(phase = EventPhase.Interrupted, stopReason = null)) to false,
                listOf(reply.copy(phase = EventPhase.Interrupted)) to true,
                listOf(reply, user, settled) to false,
            )
            for ((index, entry) in cases.withIndex()) {
                val (events, failed) = entry
                val generation = "failure-$index"
                reopened.sendMessage(TranscriptSnapshot(quiet.id, TranscriptCut(generation, 0, events, queue)))
                val current = controller.awaitState { it.transcripts[quiet.id]?.position?.generation == generation }
                val retained = current.transcripts.getValue(quiet.id)
                assertEquals(failed, retained.latestResponseFailed(), "Model failure classification for case $index")
                if (index == 1) {
                    val parts = presentTranscript(retained.rows).single().parts
                    assertTrue(parts.none { it is TranscriptPart.Failure }, "A recoverable tool error is not a failed response")
                    assertTrue(assertIs<TranscriptPart.Details>(parts.single()).blocks.single().results.single().event.isError,
                        "The tool error stays visible in Details")
                }
            }
        } finally {
            controller.dispose()
            server.stop(1_000, 1_000)
            directory.toFile().deleteRecursively()
        }
    }

    @Test
    fun warms_running_unread_and_recent_chats_without_eviction_or_read_floods() = runBlocking {
        val directory = Files.createTempDirectory("tau-warming")
        val sockets = Channel<DefaultWebSocketServerSession>(Channel.UNLIMITED)
        val requests = Channel<ClientRequest>(Channel.UNLIMITED)
        val recent = (1..5).map { chat.copy(id = "recent-$it") }
        val running = (1..3).map { chat.copy(id = "running-$it", status = SessionStatus.Running) }
        val unread = chat.copy(id = "unread")
        val sessions = listOf(chat) + recent + unread + running
        val events = (0L until 300L).map { TranscriptEvent("entry:$it:0", it, "$it", role = EventRole.User, kind = EventKind.Text, text = "Message $it") }
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                webSocket("/v1/ws") {
                    sendMessage(Hello(TauProtocolVersion, "test"))
                    sendMessage(Sessions(sessions))
                    sockets.send(this)
                    for (frame in incoming) if (frame is Frame.Text) {
                        val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                        if (request is ListSessions && request.id.startsWith("heartbeat-")) sendMessage(Response(request.id, true))
                        else requests.send(request)
                    }
                }
            }
        }.start(wait = false)
        val port = server.engine.resolvedConnectors().single().port
        val controller = TauController(Dispatchers.Swing, LocalStore({ directory.resolve("local.db").toString() }))
        try {
            withContext(Dispatchers.Swing) { controller.start(ConnectionSettings("http://127.0.0.1:$port", "test-token")) }
            val socket = withTimeout(10_000) { sockets.receive() }
            val initial = List(3) { assertIs<OpenSession>(requests.nextRequest()) }
            assertEquals(listOf(chat.id, running[0].id, running[1].id), initial.map { it.sessionId })
            assertTrue(requests.tryReceive().isFailure, "At most two background reads")
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 0, events.takeLast(50), queue, 250)))
            socket.sendMessage(Response(initial[0].id, true, chat.id))
            for (cursor in listOf(250L, 200L)) {
                val read = assertIs<GetHistory>(requests.nextRequest())
                assertEquals(chat.id, read.sessionId)
                assertEquals(cursor, read.before)
                socket.sendMessage(TranscriptPage(read.id, chat.id, "g", cursor, HistoryPage(events.subList(cursor.toInt() - 50, cursor.toInt()), cursor - 50)))
                socket.sendMessage(Response(read.id, true, chat.id))
            }
            controller.awaitState { it.transcripts[chat.id]?.rows?.size == 150 }
            assertTrue(requests.tryReceive().isFailure, "Warming stops at its window, not all history")
            socket.sendMessage(Response(initial[1].id, false, running[0].id, error = "Unavailable history"))
            val third = assertIs<OpenSession>(requests.nextRequest())
            assertEquals(running[2].id, third.sessionId)
            assertNull(controller.state.value.error, "A failed background read stays out of the selected chat")
            socket.sendMessage(Sessions(sessions.map { if (it.id == unread.id) it.copy(updatedAtMs = 2) else it }))
            controller.awaitState { unread.id in it.unread }
            socket.sendMessage(TranscriptSnapshot(running[1].id, TranscriptCut("g", 0, emptyList(), queue)))
            socket.sendMessage(Response(initial[2].id, true, running[1].id))
            val unreadOpen = assertIs<OpenSession>(requests.nextRequest())
            assertEquals(unread.id, unreadOpen.sessionId)
            socket.sendMessage(TranscriptSnapshot(unread.id, TranscriptCut("g", 0, events.takeLast(50), queue, 250)))
            socket.sendMessage(Response(unreadOpen.id, true, unread.id))
            for (cursor in listOf(250L, 200L)) {
                val read = assertIs<GetHistory>(requests.nextRequest())
                assertEquals(unread.id, read.sessionId)
                socket.sendMessage(TranscriptPage(read.id, unread.id, "g", cursor, HistoryPage(events.subList(cursor.toInt() - 50, cursor.toInt()), cursor - 50)))
                socket.sendMessage(Response(read.id, true, unread.id))
            }
            val firstRecent = assertIs<OpenSession>(requests.nextRequest())
            assertEquals(recent[0].id, firstRecent.sessionId)
            socket.sendMessage(TranscriptSnapshot(running[2].id, TranscriptCut("g", 0, emptyList(), queue)))
            socket.sendMessage(Response(third.id, true, running[2].id))
            val secondRecent = assertIs<OpenSession>(requests.nextRequest())
            assertEquals(recent[1].id, secondRecent.sessionId)
            assertTrue(unread.id in controller.state.value.unread, "Background warming never marks a chat read")
            val before = controller.state.value.transcripts.getValue(chat.id).rows.toList()
            withContext(Dispatchers.Swing) { controller.selectSession(unread.id) }
            controller.awaitState { it.selectedSessionId == unread.id && it.transcripts[unread.id]?.synchronized == true }
            assertEquals(150, controller.state.value.transcripts.getValue(unread.id).rows.size)
            assertTrue(unread.id !in controller.state.value.unread)
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 1, TranscriptChange(events = listOf(events.last().copy(text = "Updated while away")))))
            controller.awaitState { it.transcripts[chat.id]?.position?.sequence == 1L }
            withContext(Dispatchers.Swing) { controller.selectSession(chat.id) }
            controller.awaitState { it.selectedSessionId == chat.id }
            delay(100)
            assertEquals(before, controller.state.value.transcripts.getValue(chat.id).rows.toList())
            assertEquals("Updated while away", before.last().event.text)
            assertTrue(requests.tryReceive().isFailure, "Switching keeps rows and feeds; failed background reads do not loop")
        } finally {
            withContext(Dispatchers.Swing) { controller.dispose() }.join()
            server.stop(0, 1000)
            directory.toFile().deleteRecursively()
        }
    }

    @Test
    fun retains_ordered_content_queue_intents_and_local_work_across_socket_and_process_loss() = runBlocking {
        val directory = Files.createTempDirectory("tau-controller")
        val path = directory.resolve("transcript.db").toString()
        val other = chat.copy(id = "other", title = "Other chat")
        val sockets = Channel<DefaultWebSocketServerSession>(Channel.UNLIMITED)
        val requests = Channel<ClientRequest>(Channel.UNLIMITED)
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                webSocket("/v1/ws") {
                    assertEquals("Bearer test-token", call.request.headers[HttpHeaders.Authorization])
                    sendMessage(Hello(TauProtocolVersion, "test"))
                    sendMessage(Sessions(listOf(chat, other)))
                    sockets.send(this)
                    for (frame in incoming) if (frame is Frame.Text) {
                        val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                        if (request is ListSessions && request.id.startsWith("heartbeat-")) sendMessage(Response(request.id, true))
                        else requests.send(request)
                    }
                }
            }
        }.start(wait = false)
        val port = server.engine.resolvedConnectors().single().port
        val settings = ConnectionSettings("http://127.0.0.1:$port", "test-token")
        var controller = TauController(Dispatchers.Swing, LocalStore({ path }))
        try {
            withContext(Dispatchers.Swing) { controller.start(settings) }
            var socket = withTimeout(10_000) { sockets.receive() }
            val open = assertIs<OpenSession>(requests.nextRequest())
            val user = TranscriptEvent("entry:u0:0", 0, "u0", role = EventRole.User, kind = EventKind.Text, text = "Start")
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 0, listOf(user), queue)))
            socket.sendMessage(Response(open.id, true, chat.id))
            controller.awaitState { it.transcripts[chat.id]?.synchronized == true }
            assertEquals(chat.contextUsage, controller.state.value.sessions.first { it.id == chat.id }.contextUsage)
            socket.sendMessage(SessionState(chat.id, SessionStatus.Idle, contextUsage = ContextUsage(null, 128000)))
            controller.awaitState { it.sessions.first { session -> session.id == chat.id }.contextUsage == ContextUsage(null, 128000) }
            socket.sendMessage(SessionState(chat.id, SessionStatus.Idle))
            controller.awaitState { it.sessions.first { session -> session.id == chat.id }.contextUsage == null }
            socket.sendMessage(SessionState(chat.id, SessionStatus.Idle, contextUsage = ContextUsage(96000, 128000)))
            controller.awaitState { it.sessions.first { session -> session.id == chat.id }.contextUsage == ContextUsage(96000, 128000) }

            withContext(Dispatchers.Swing) { controller.selectSession(other.id) }
            val openOther = assertIs<OpenSession>(requests.nextRequest())
            socket.sendMessage(TranscriptSnapshot(other.id, TranscriptCut("other", 0, events = emptyList(), queue = queue)))
            socket.sendMessage(Response(openOther.id, true, other.id))
            controller.awaitState { it.transcripts[other.id]?.synchronized == true }
            val retainedUser = controller.state.value.transcripts.getValue(chat.id).rows.single()
            withContext(Dispatchers.Swing) { controller.selectSession(chat.id) }
            assertSame(retainedUser, controller.state.value.transcripts.getValue(chat.id).rows.single())
            assertTrue(requests.tryReceive().isFailure, "Switching back keeps its current feed")
            socket.sendMessage(TranscriptSnapshot(other.id, TranscriptCut("other", 1, listOf(user), queue)))
            controller.awaitState { it.transcripts[other.id]?.rows?.size == 1 }
            socket.sendMessage(TranscriptUpdate(other.id, "other", 99, TranscriptChange(removed = listOf(user.id))))
            val otherRecovery = assertIs<OpenSession>(requests.nextRequest())
            assertEquals(other.id, otherRecovery.sessionId)
            socket.sendMessage(ResyncRequired(other.id))
            socket.sendMessage(ResyncRequired())
            val refreshList = assertIs<ListSessions>(requests.nextRequest())
            val scopedOpen = assertIs<OpenSession>(requests.nextRequest())
            assertEquals(chat.id, scopedOpen.sessionId)
            socket.sendMessage(Response(refreshList.id, true))
            socket.sendMessage(ResyncRequired(chat.id))
            socket.sendMessage(SessionState(other.id, SessionStatus.Idle, detail = "Scope barrier"))
            controller.awaitState { it.sessions.any { session -> session.detail == "Scope barrier" } }
            assertEquals(listOf(user), controller.state.value.transcripts.getValue(other.id).rows.map { it.event })
            assertTrue(requests.tryReceive().isFailure, "Recovery opens each retained chat once")
            socket.sendMessage(TranscriptSnapshot(other.id, TranscriptCut("other", 1, listOf(user), queue)))
            socket.sendMessage(Response(otherRecovery.id, true, other.id))
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 0, listOf(user), queue)))
            socket.sendMessage(Response(scopedOpen.id, true, chat.id))
            controller.awaitState { it.transcripts[chat.id]?.synchronized == true }

            withContext(Dispatchers.Swing) { controller.setDraft(chat.id, "Repeat"); controller.sendPrompt() }
            val first = assertIs<Prompt>(requests.nextRequest())
            controller.awaitState { chat.id !in it.uploadingSessions }
            withContext(Dispatchers.Swing) { controller.setDraft(chat.id, "Repeat"); controller.sendPrompt() }
            val second = assertIs<Prompt>(requests.nextRequest())
            assertNotEquals(first.id, second.id)
            assertEquals(first.text, second.text)
            val queued = queue.copy(requests = listOf(
                QueuedRequest(second.id, 0, "steer", "Prepared repeat"),
                QueuedRequest(first.id, 0, "followUp", "Prepared repeat"),
            ))
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 1, TranscriptChange(queue = queued)))
            socket.sendMessage(Response(first.id, true, chat.id, disposition = "queued"))
            socket.sendMessage(Response(second.id, false, chat.id, uncertain = true))
            val accepted = controller.awaitState { it.transcripts[chat.id]?.pending?.all { send -> send.status == SendStatus.Queued } == true }
            val retained = accepted.transcripts.getValue(chat.id)
            assertEquals(listOf(second.id, first.id), retained.pending.map { it.requestId })
            assertEquals("Prepared repeat", retained.pending.first().text)
            val selection = QueueOperation.Prefix("run", listOf(QueueRef(second.id, 0)), "reasoning_checkpoint")
            withContext(Dispatchers.Swing) { controller.queueControl(chat.id, "g", selection) }
            val prefix = assertIs<ControlQueue>(requests.nextRequest())
            assertEquals(selection, prefix.operation)
            val waiting = QueueControl(prefix.id, "run", "prefix", "reasoning_checkpoint", selection.requests, "waiting")
            var currentQueue = queued.copy(control = waiting)
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 2, TranscriptChange(queue = currentQueue)))
            socket.sendMessage(Response(prefix.id, true, chat.id, outcome = "accepted"))
            currentQueue = currentQueue.copy(requests = currentQueue.requests + QueuedRequest("later", 0, "followUp", "Later arrival"))
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 3, TranscriptChange(queue = currentQueue)))
            controller.awaitState { it.transcripts[chat.id]?.pending?.size == 3 }
            assertEquals(selection.requests, retained.queue.control?.requests)

            var live = TranscriptEvent("stream:a:0", 1, "live-a", phase = EventPhase.Live, role = EventRole.Assistant,
                origin = EventOrigin(streamId = "a"), kind = EventKind.Thinking, text = "Retained")
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 4, TranscriptChange(events = listOf(live))))
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 5, TranscriptChange(delta = TextDelta(live.id, " thinking"))))
            controller.awaitState { it.transcripts[chat.id]?.rows?.lastOrNull()?.event?.text == "Retained thinking" }
            val row = retained.rows.last()
            withContext(Dispatchers.Swing) { controller.setExpanded(chat.id, "details:${row.key}", true) }
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 7, TranscriptChange(delta = TextDelta(live.id, "MUST NOT APPLY"))))
            val recovery = assertIs<OpenSession>(requests.nextRequest())
            assertEquals("Retained thinking", row.event.text)
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 4, listOf(user, live), currentQueue)))
            val fresh = assertIs<OpenSession>(requests.nextRequest())
            assertEquals("Retained thinking", row.event.text)
            live = live.copy(text = "Retained thinking through the gap")
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 7, listOf(user, live), currentQueue)))
            socket.sendMessage(Response(recovery.id, true, chat.id))
            socket.sendMessage(Response(fresh.id, true, chat.id))
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 7, TranscriptChange(delta = TextDelta(live.id, "DUPLICATE"))))
            val saved = live.copy(entryId = "a1", phase = EventPhase.Saved, stopReason = "error", errorMessage = "Recovery attempt")
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 8, TranscriptChange(events = listOf(saved))))
            controller.awaitState { it.transcripts[chat.id]?.rows?.lastOrNull()?.event?.phase == EventPhase.Saved }
            assertSame(row, retained.rows.last())
            assertEquals("Retained thinking through the gap", row.event.text)
            assertEquals("true", retained.preferences["expanded:details:${row.key}"])

            val interrupted = TranscriptEvent("stream:b:0", 2, "live-b", phase = EventPhase.Live, role = EventRole.Assistant,
                origin = EventOrigin(streamId = "b"), kind = EventKind.Thinking, text = "Unfinished work")
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 9, TranscriptChange(events = listOf(interrupted))))
            controller.awaitState { it.transcripts[chat.id]?.rows?.size == 3 }
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("replacement", 0, listOf(user, saved, interrupted.copy(phase = EventPhase.Interrupted)), queue.copy(runId = null))))
            controller.awaitState { it.transcripts[chat.id]?.rows?.lastOrNull()?.event?.phase == EventPhase.Interrupted }
            assertEquals("unconfirmed", retained.controls.single().status)
            assertTrue(retained.pending.all { it.status == SendStatus.Unconfirmed })

            withContext(Dispatchers.Swing) { controller.setDraft(chat.id, "/model astra") }
            val abandonedCommands = assertIs<GetCommands>(requests.nextRequest())
            socket.close(CloseReason(CloseReason.Codes.GOING_AWAY, "Command response lost"))
            controller.awaitState { it.connectionStatus == ConnectionStatus.Offline }
            socket = withTimeout(10_000) { sockets.receive() }
            val commandRecovery = assertIs<OpenSession>(requests.nextRequest())
            val retriedCommands = assertIs<GetCommands>(requests.nextRequest())
            val otherReconnect = assertIs<OpenSession>(requests.nextRequest())
            assertEquals(other.id, otherReconnect.sessionId)
            socket.sendMessage(TranscriptSnapshot(other.id, TranscriptCut("other", 1, listOf(user), queue)))
            socket.sendMessage(Response(otherReconnect.id, true, other.id))
            assertNotEquals(abandonedCommands.id, retriedCommands.id)
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("replacement", 0, listOf(user, saved, interrupted.copy(phase = EventPhase.Interrupted)), queue.copy(runId = null))))
            socket.sendMessage(Response(commandRecovery.id, true, chat.id))
            socket.sendMessage(SessionState(chat.id, SessionStatus.Idle, contextUsage = ContextUsage(96000, 128000)))
            val modelCommands = listOf(SlashCommand("model", source = SlashCommandSource.Builtin,
                arguments = listOf(SlashCommandArgument("openai-codex/gpt-6-astra", "GPT-6 Astra"))))
            socket.sendMessage(Commands(chat.id, modelCommands))
            socket.sendMessage(Response(retriedCommands.id, true, chat.id))
            controller.awaitState { it.slashCommands[chat.id] == modelCommands && chat.id !in it.loadingCommands }
            assertEquals("/model astra", controller.state.value.drafts[chat.id])
            assertTrue(requests.tryReceive().isFailure, "Only read-only command loading is retried")
            socket.sendMessage(SessionState(chat.id, SessionStatus.Sleeping, contextUsage = ContextUsage(96000, 128000)))
            controller.awaitState { chat.id !in it.slashCommands }
            withContext(Dispatchers.Swing) { controller.setDraft(chat.id, "/model astra ") }
            val delayedCommands = assertIs<GetCommands>(requests.nextRequest())
            assertNull(controller.awaitState(15_000) { chat.id !in it.loadingCommands }.error,
                "Automatic command loading produced a timeout banner")
            socket.sendMessage(Commands(chat.id, modelCommands))
            socket.sendMessage(Response(delayedCommands.id, true, chat.id))
            controller.awaitState { it.slashCommands[chat.id] == modelCommands }

            withContext(Dispatchers.Swing) { controller.setDraft(chat.id, "No acknowledgement"); controller.sendPrompt() }
            val unacknowledged = assertIs<Prompt>(requests.nextRequest())
            controller.awaitState { chat.id !in it.uploadingSessions }
            withContext(Dispatchers.Swing) {
                controller.setDraft(chat.id, "Next local draft")
                controller.attachClipboardImage { PickedFile("retained.png", byteArrayOf(1, 2, 3, 4)) }
                controller.saveScroll(chat.id, ScrollPosition(row.key, 27, false))
                controller.start(settings)
            }
            controller.awaitState { it.transcripts[chat.id]?.files?.size == 1 && it.transcripts[chat.id]?.preferences?.get("scroll") != null }
            socket.close(CloseReason(CloseReason.Codes.GOING_AWAY, "Outage"))
            controller.awaitState { it.connectionStatus == ConnectionStatus.Offline }
            assertEquals(SendStatus.Unconfirmed, retained.pending.first { it.requestId == unacknowledged.id }.status)
            withContext(Dispatchers.Swing) {
                controller.setDraft(chat.id, "Final edit before closing")
                controller.dispose()
            }.join()
            server.stop(0, 1_000)
            controller = TauController(Dispatchers.Swing, LocalStore({ path }))
            withContext(Dispatchers.Swing) { controller.start(settings) }
            val restored = controller.awaitState { !it.restoring && it.transcripts[chat.id] != null }
            val reopened = restored.transcripts.getValue(chat.id)
            assertEquals("Final edit before closing", restored.drafts[chat.id])
            assertEquals(ContextUsage(96000, 128000), restored.sessions.first { it.id == chat.id }.contextUsage)
            assertEquals("retained.png", reopened.files.single().name)
            assertTrue(reopened.rows.isEmpty())
            assertEquals("true", reopened.preferences["expanded:details:${row.key}"])
            assertEquals(ScrollPosition(row.key, 27, false), TauJson.decodeFromString<ScrollPosition>(reopened.preferences.getValue("scroll")))
            assertEquals(selection, reopened.controls.single().operation)
            assertTrue(reopened.pending.all { it.status == SendStatus.Unconfirmed })
            assertTrue(requests.tryReceive().isFailure, "Recovery and recreation never replay prompts or controls")
        } finally {
            withContext(Dispatchers.Swing) { controller.dispose() }.join()
            server.stop(0, 1_000)
            sockets.close(); requests.close()
            directory.toFile().deleteRecursively()
        }
    }

    @Test
    fun pages_flat_history_with_live_updates_and_stale_replies(): Unit = runBlocking {
        val directory = Files.createTempDirectory("tau-controller")
        val path = directory.resolve("transcript.db").toString()
        val other = chat.copy(id = "other", title = "Other chat")
        val sockets = Channel<DefaultWebSocketServerSession>(Channel.UNLIMITED)
        val requests = Channel<ClientRequest>(Channel.UNLIMITED)
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                webSocket("/v1/ws") {
                    assertEquals("Bearer test-token", call.request.headers[HttpHeaders.Authorization])
                    sendMessage(Hello(TauProtocolVersion, "test"))
                    sendMessage(Sessions(listOf(chat, other)))
                    sockets.send(this)
                    for (frame in incoming) if (frame is Frame.Text) {
                        val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                        if (request is ListSessions && request.id.startsWith("heartbeat-")) sendMessage(Response(request.id, true))
                        else requests.send(request)
                    }
                }
            }
        }.start(wait = false)
        val port = server.engine.resolvedConnectors().single().port
        val settings = ConnectionSettings("http://127.0.0.1:$port", "test-token")
        val controller = TauController(Dispatchers.Swing, LocalStore({ path }))
        try {
            withContext(Dispatchers.Swing) { controller.start(settings) }
            var socket = withTimeout(10_000) { sockets.receive() }
            val initial = assertIs<OpenSession>(requests.nextRequest())
            val warmOther = assertIs<OpenSession>(requests.nextRequest())
            assertEquals(other.id, warmOther.sessionId)
            socket.sendMessage(TranscriptSnapshot(other.id, TranscriptCut("other", 0, events = emptyList(), queue = queue)))
            socket.sendMessage(Response(warmOther.id, true, other.id))
            val user = TranscriptEvent("entry:u0:0", 0, "u0", role = EventRole.User, kind = EventKind.Text, text = "Start")
            val old = TranscriptEvent("entry:history:0", 0, "history", role = EventRole.User, kind = EventKind.Text, text = "Older history")
            val recent = user.copy(id = "entry:recent:0", entryId = "recent", order = 1)
            val paged = TranscriptCut("pages", 0, listOf(recent), queue, recent.order)
            socket.sendMessage(TranscriptSnapshot(chat.id, paged))
            socket.sendMessage(Response(initial.id, true, chat.id))
            controller.awaitState { it.transcripts[chat.id]?.before == recent.order }
            withContext(Dispatchers.Swing) { controller.loadOlder(chat.id); controller.loadOlder(chat.id) }
            val stalePage = assertIs<GetHistory>(requests.nextRequest())
            assertEquals(recent.order, stalePage.before)
            assertTrue(requests.tryReceive().isFailure, "One history request at a time")
            socket.sendMessage(ResyncRequired(chat.id))
            val refreshPage = assertIs<OpenSession>(requests.nextRequest())
            withContext(Dispatchers.Swing) { controller.loadOlder(chat.id); controller.loadOlder(chat.id) }
            assertNull(controller.state.value.error, "Refreshing a chat is not an offline error")
            assertTrue(chat.id in controller.state.value.loadingHistory)
            assertTrue(requests.tryReceive().isFailure, "Wait for the existing open instead of sending a stale cursor")
            socket.sendMessage(TranscriptSnapshot(chat.id, paged))
            socket.sendMessage(Response(refreshPage.id, true, chat.id))
            socket.sendMessage(TranscriptPage(stalePage.id, chat.id, "pages", recent.order, HistoryPage(listOf(old))))
            socket.sendMessage(SessionState(other.id, SessionStatus.Idle, detail = "Stale page barrier"))
            controller.awaitState { it.sessions.any { session -> session.detail == "Stale page barrier" } }
            assertEquals(1, controller.state.value.transcripts.getValue(chat.id).rows.size)
            val historyRequest = assertIs<GetHistory>(requests.nextRequest())
            val pagingLive = TranscriptEvent("stream:paging:0", 2, "live-paging", phase = EventPhase.Live, origin = EventOrigin(streamId = "paging"),
                role = EventRole.Assistant, kind = EventKind.Thinking, text = "π")
            socket.sendMessage(TranscriptUpdate(chat.id, "pages", 1, TranscriptChange(events = listOf(pagingLive))))
            socket.sendMessage(TranscriptUpdate(chat.id, "pages", 2, TranscriptChange(delta = TextDelta(pagingLive.id, "🧠"))))
            socket.sendMessage(TranscriptPage(historyRequest.id, chat.id, "pages", recent.order, HistoryPage(listOf(old))))
            socket.sendMessage(Response(historyRequest.id, true, chat.id))
            controller.awaitState { it.transcripts[chat.id]?.before == null && it.transcripts[chat.id]?.position?.sequence == 2L }
            assertEquals("π🧠", controller.state.value.transcripts.getValue(chat.id).rows.last().event.text)
            val retainedRows = controller.state.value.transcripts.getValue(chat.id).rows.toList()
            withContext(Dispatchers.Swing) { controller.selectSession(other.id) }
            socket.sendMessage(TranscriptUpdate(chat.id, "pages", 3, TranscriptChange(delta = TextDelta(pagingLive.id, " while away"))))
            controller.awaitState { it.transcripts[chat.id]?.position?.sequence == 3L }
            withContext(Dispatchers.Swing) { controller.selectSession(chat.id) }
            val kept = controller.state.value.transcripts.getValue(chat.id)
            assertTrue(kept.synchronized)
            assertEquals(retainedRows, kept.rows.toList())
            assertEquals("π🧠 while away", kept.rows.last().event.text)
            withContext(Dispatchers.Swing) { controller.loadOlder(chat.id) }
            controller.awaitState { it.transcripts[chat.id]?.before == null && chat.id !in it.loadingHistory }
            assertTrue(requests.tryReceive().isFailure, "Loaded history remains available while retained in memory")
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 0, listOf(user), queue)))
            controller.awaitState { it.transcripts[chat.id]?.position?.generation == "g" }

            socket.sendMessage(TranscriptSnapshot(chat.id, paged.copy(generation = "reconnect")))
            controller.awaitState { it.transcripts[chat.id]?.position?.generation == "reconnect" }
            assertIs<GetHistory>(requests.nextRequest())
            socket.close(CloseReason(CloseReason.Codes.NORMAL, "Reconnect with an unloaded page"))
            controller.awaitState { it.connectionStatus == ConnectionStatus.Offline }
            withContext(Dispatchers.Swing) { controller.loadOlder(chat.id); controller.loadOlder(chat.id) }
            assertNull(controller.state.value.error)
            assertTrue(chat.id in controller.state.value.loadingHistory)
            socket = withTimeout(10_000) { sockets.receive() }
            val reconnect = assertIs<OpenSession>(requests.nextRequest())
            val otherReconnect = assertIs<OpenSession>(requests.nextRequest())
            assertEquals(other.id, otherReconnect.sessionId)
            socket.sendMessage(TranscriptSnapshot(other.id, TranscriptCut("other", 0, events = emptyList(), queue = queue)))
            socket.sendMessage(Response(otherReconnect.id, true, other.id))
            assertTrue(requests.tryReceive().isFailure, "History waits for the new snapshot")
            socket.sendMessage(TranscriptSnapshot(chat.id, paged.copy(generation = "reconnect")))
            socket.sendMessage(Response(reconnect.id, true, chat.id))
            val resumed = assertIs<GetHistory>(requests.nextRequest())
            assertEquals("reconnect", resumed.generation)
            assertEquals(recent.order, resumed.before)
            assertTrue(requests.tryReceive().isFailure, "Only the history read resumes")
            socket.sendMessage(TranscriptPage(resumed.id, chat.id, resumed.generation, resumed.before, HistoryPage(listOf(old))))
            socket.sendMessage(Response(resumed.id, true, chat.id))
            controller.awaitState { it.transcripts[chat.id]?.before == null && chat.id !in it.loadingHistory }
            assertEquals(listOf(old, recent), controller.state.value.transcripts.getValue(chat.id).rows.map { it.event })

        } finally {
            withContext(Dispatchers.Swing) { controller.dispose() }.join()
            server.stop(0, 1000)
            directory.toFile().deleteRecursively()
        }
    }

    @Test
    fun keeps_reconnect_failures_quiet_but_reports_actionable_errors() = runBlocking {
        for (failure in listOf(java.net.UnknownHostException("fixture"), java.nio.channels.UnresolvedAddressException(),
            java.net.SocketException("Software caused connection abort"), TauConnectionException("Tau is not responding"))) {
            assertTrue(failure.isConnectionFailure())
            assertTrue(Exception("Wrapped failure", failure).isConnectionFailure())
        }
        assertFalse(IllegalStateException("Invalid transcript").isConnectionFailure())
        val directory = Files.createTempDirectory("tau-quiet-reconnect")
        val port = java.net.ServerSocket(0).use { it.localPort }
        val protocol = AtomicInteger(TauProtocolVersion + 1)
        val connections = AtomicInteger()
        val server = embeddedServer(CIO, host = "127.0.0.1", port = port) {
            install(WebSockets)
            routing {
                webSocket("/v1/ws") {
                    connections.incrementAndGet()
                    sendMessage(Hello(protocol.get(), "fixture"))
                    sendMessage(Sessions(listOf(chat)))
                    for (frame in incoming) if (frame is Frame.Text) {
                        val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                        when (request) {
                            is OpenSession -> {
                                sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 0,
                                    listOf(TranscriptEvent("user:0", 1, "user", role = EventRole.User, kind = EventKind.Text, text = "Kept history")), queue, 1)))
                                sendMessage(Response(request.id, true, chat.id))
                            }
                            is CreateSession -> sendMessage(Response(request.id, false, error = "Create rejected"))
                            is GetHistory -> Unit
                            else -> sendMessage(Response(request.id, true))
                        }
                    }
                }
            }
        }
        val controller = TauController(Dispatchers.Swing, LocalStore({ directory.resolve("local.db").toString() }))
        try {
            withContext(Dispatchers.Swing) { controller.start(ConnectionSettings("http://127.0.0.1:$port", "fixture")) }
            assertNull(controller.awaitState { it.connectionStatus == ConnectionStatus.Offline }.error,
                "A routine connection failure produced a banner")
            delay(2_500)
            assertNull(controller.awaitState { it.connectionStatus == ConnectionStatus.Offline }.error,
                "A retry repeated the network banner")
            server.start(wait = false)
            controller.awaitState { it.error?.contains("matching client and daemon update") == true }
            protocol.set(TauProtocolVersion)
            controller.awaitState { it.connectionStatus == ConnectionStatus.Connected && it.transcripts[chat.id]?.synchronized == true }
            assertNull(controller.state.value.error)
            assertTrue(connections.get() >= 2)
            controller.awaitState { chat.id in it.loadingHistory }
            assertNull(controller.awaitState(15_000) { chat.id !in it.loadingHistory }.error,
                "Automatic history loading produced a timeout banner")
            assertEquals(1L, controller.state.value.transcripts.getValue(chat.id).before)
            withContext(Dispatchers.Swing) { controller.createSession() }
            controller.awaitState { it.error == "Create rejected" }
            withContext(Dispatchers.Swing) { controller.dismissError(); controller.setDraft(chat.id, "Keep this draft") }
            server.stop(0, 1000)
            assertNull(controller.awaitState { it.connectionStatus == ConnectionStatus.Offline }.error)
            delay(2_500)
            val offline = controller.awaitState { it.connectionStatus == ConnectionStatus.Offline }
            assertNull(offline.error, "A lost connection produced a banner")
            assertEquals("Keep this draft", offline.drafts[chat.id])
        } finally {
            withContext(Dispatchers.Swing) { controller.dispose() }.join()
            server.stop(0, 1000)
            directory.toFile().deleteRecursively()
        }
    }

    @Test
    fun reconnects_when_commands_arrive_but_the_return_path_stops() = runBlocking {
        val directory = Files.createTempDirectory("tau-heartbeat")
        val connections = AtomicInteger()
        val applied = AtomicBoolean()
        val heartbeats = AtomicInteger()
        val requests = Channel<ClientRequest>(Channel.UNLIMITED)
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                webSocketRaw("/v1/ws") {
                    val connection = connections.incrementAndGet()
                    sendMessage(Hello(TauProtocolVersion, "test-$connection"))
                    sendMessage(Sessions(if (applied.get()) listOf(chat) else emptyList()))
                    for (frame in incoming) when (frame) {
                        is Frame.Text -> {
                            val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                            if (request is ListSessions && request.id.startsWith("heartbeat-")) {
                                if (connection > 1) {
                                    heartbeats.incrementAndGet()
                                    sendMessage(Sessions(listOf(chat)))
                                    sendMessage(Response(request.id, true))
                                }
                                continue
                            }
                            requests.send(request)
                            when (request) {
                                is CreateSession -> applied.set(true)
                                is OpenSession -> {
                                    sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 0, events = emptyList(), queue = queue)))
                                    sendMessage(Response(request.id, true, chat.id))
                                }
                                else -> Unit
                            }
                        }
                        else -> Unit
                    }
                }
            }
        }.start(wait = false)
        val port = server.engine.resolvedConnectors().single().port
        val controller = TauController(Dispatchers.Swing, LocalStore({ directory.resolve("transcript.db").toString() }))
        try {
            withContext(Dispatchers.Swing) { controller.start(ConnectionSettings("http://127.0.0.1:$port", "test-token")) }
            controller.awaitState { it.connectionStatus == ConnectionStatus.Connected }
            withContext(Dispatchers.Swing) { controller.createSession() }
            assertIs<CreateSession>(requests.nextRequest())
            controller.awaitState(60_000) { it.connectionStatus == ConnectionStatus.Connected && it.daemonVersion != "test-1" && it.transcripts[chat.id]?.synchronized == true }
            assertTrue(applied.get())
            assertTrue(connections.get() >= 2)
            assertIs<OpenSession>(requests.nextRequest())
            delay(TauHeartbeatMillis * 4)
            assertEquals(2, connections.get(), "Normal replies keep the connection healthy without control-frame pongs")
            assertTrue(heartbeats.get() >= 3)
            assertEquals(ConnectionStatus.Connected, controller.state.value.connectionStatus)
            assertTrue(requests.tryReceive().isFailure, "Reconnect must not repeat the applied command")
        } finally {
            withContext(Dispatchers.Swing) { controller.dispose() }.join()
            server.stop(0, 1_000)
            requests.close()
            directory.toFile().deleteRecursively()
        }
    }
}
