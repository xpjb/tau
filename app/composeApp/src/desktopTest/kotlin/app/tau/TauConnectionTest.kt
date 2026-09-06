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
import kotlin.test.assertIs
import kotlin.test.assertNotEquals
import kotlin.test.assertSame
import kotlin.test.assertTrue

class TauConnectionTest {
    private val chat = SessionSummary("chat", "Test chat", SessionStatus.Idle, createdAtMs = 1, updatedAtMs = 1,
        contextUsage = ContextUsage(64000, 200000))
    private val queue = QueueState(available = true, runId = "run",
        capabilities = listOf("queue_edit", "queue_delete", "queue_run_prefix", "queue_resume", "queue_cancel_control"),
        boundaries = listOf("reasoning_checkpoint", "turn"))

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
    fun retains_ordered_content_queue_intents_and_local_work_across_socket_and_process_loss() = runBlocking {
        val directory = Files.createTempDirectory("tau-controller")
        val path = directory.resolve("transcript.db").toString()
        val sockets = Channel<DefaultWebSocketServerSession>(Channel.UNLIMITED)
        val requests = Channel<ClientRequest>(Channel.UNLIMITED)
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                webSocket("/v1/ws") {
                    assertEquals("Bearer test-token", call.request.headers[HttpHeaders.Authorization])
                    sendMessage(Hello(TauProtocolVersion, "test"))
                    sendMessage(Sessions(listOf(chat)))
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
        var controller = TauController(Dispatchers.Swing, TranscriptStore({ path }))
        try {
            withContext(Dispatchers.Swing) { controller.start(settings) }
            var socket = withTimeout(10_000) { sockets.receive() }
            val open = assertIs<OpenSession>(requests.nextRequest())
            val user = TranscriptEntry("u0", role = EntryRole.User, content = listOf(EntryContent(ContentKind.Text, "Start")))
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 0, user.id, listOf(user), queue)))
            socket.sendMessage(Response(open.id, true, chat.id))
            controller.awaitState { it.transcripts[chat.id]?.synchronized == true }
            assertEquals(chat.contextUsage, controller.state.value.sessions.single().contextUsage)
            socket.sendMessage(SessionState(chat.id, SessionStatus.Idle, contextUsage = ContextUsage(null, 128000)))
            controller.awaitState { it.sessions.single().contextUsage == ContextUsage(null, 128000) }
            socket.sendMessage(SessionState(chat.id, SessionStatus.Idle))
            controller.awaitState { it.sessions.single().contextUsage == null }
            socket.sendMessage(SessionState(chat.id, SessionStatus.Idle, contextUsage = ContextUsage(96000, 128000)))
            controller.awaitState { it.sessions.single().contextUsage == ContextUsage(96000, 128000) }

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
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 1, TranscriptChange.Queue(queued)))
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
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 2, TranscriptChange.Queue(currentQueue)))
            socket.sendMessage(Response(prefix.id, true, chat.id, outcome = "accepted"))
            currentQueue = currentQueue.copy(requests = currentQueue.requests + QueuedRequest("later", 0, "followUp", "Later arrival"))
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 3, TranscriptChange.Queue(currentQueue)))
            controller.awaitState { it.transcripts[chat.id]?.pending?.size == 3 }
            assertEquals(selection.requests, retained.queue.control?.requests)

            var live = TranscriptEntry("live-a", user.id, phase = EntryPhase.Live, role = EntryRole.Assistant,
                origin = EntryOrigin(streamId = "a"), content = listOf(EntryContent(ContentKind.Thinking, "Retained")))
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 4, TranscriptChange.Entry(live)))
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 5, TranscriptChange.Delta(live.id, 0, " thinking")))
            controller.awaitState { it.transcripts[chat.id]?.rows?.lastOrNull()?.entry?.content?.firstOrNull()?.text == "Retained thinking" }
            val row = retained.rows.last()
            withContext(Dispatchers.Swing) { controller.setExpanded(chat.id, "details:${row.key}", true) }
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 7, TranscriptChange.Delta(live.id, 0, "MUST NOT APPLY")))
            val recovery = assertIs<OpenSession>(requests.nextRequest())
            assertEquals("Retained thinking", row.entry.content.single().text)
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 4, user.id, listOf(user, live), currentQueue)))
            val fresh = assertIs<OpenSession>(requests.nextRequest())
            assertEquals("Retained thinking", row.entry.content.single().text)
            live = live.copy(content = listOf(EntryContent(ContentKind.Thinking, "Retained thinking through the gap")))
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 7, user.id, listOf(user, live), currentQueue)))
            socket.sendMessage(Response(recovery.id, true, chat.id))
            socket.sendMessage(Response(fresh.id, true, chat.id))
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 7, TranscriptChange.Delta(live.id, 0, "DUPLICATE")))
            val saved = live.copy(id = "a1", phase = EntryPhase.Saved, stopReason = "error", errorMessage = "Recovery attempt")
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 8, TranscriptChange.Entry(saved)))
            controller.awaitState { it.transcripts[chat.id]?.rows?.lastOrNull()?.entry?.id == saved.id }
            assertSame(row, retained.rows.last())
            assertEquals("Retained thinking through the gap", row.entry.content.single().text)
            assertEquals("true", retained.preferences["expanded:details:${row.key}"])

            val interrupted = TranscriptEntry("live-b", saved.id, phase = EntryPhase.Live, role = EntryRole.Assistant,
                origin = EntryOrigin(streamId = "b"), content = listOf(EntryContent(ContentKind.Thinking, "Unfinished work")))
            socket.sendMessage(TranscriptUpdate(chat.id, "g", 9, TranscriptChange.Entry(interrupted)))
            controller.awaitState { it.transcripts[chat.id]?.rows?.size == 3 }
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("replacement", 0, saved.id, listOf(user, saved), queue.copy(runId = null))))
            controller.awaitState { it.transcripts[chat.id]?.rows?.lastOrNull()?.entry?.phase == EntryPhase.Interrupted }
            assertEquals("unconfirmed", retained.controls.single().status)
            assertTrue(retained.pending.all { it.status == SendStatus.Unconfirmed })

            withContext(Dispatchers.Swing) { controller.setDraft(chat.id, "/model astra") }
            val abandonedCommands = assertIs<GetCommands>(requests.nextRequest())
            socket.close(CloseReason(CloseReason.Codes.GOING_AWAY, "Command response lost"))
            controller.awaitState { it.connectionStatus == ConnectionStatus.Offline }
            socket = withTimeout(10_000) { sockets.receive() }
            val commandRecovery = assertIs<OpenSession>(requests.nextRequest())
            val retriedCommands = assertIs<GetCommands>(requests.nextRequest())
            assertNotEquals(abandonedCommands.id, retriedCommands.id)
            socket.sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("replacement", 0, saved.id, listOf(user, saved), queue.copy(runId = null))))
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
            controller.awaitState(15_000) { chat.id !in it.loadingCommands && it.error == "Pi command list timed out" }
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
            controller = TauController(Dispatchers.Swing, TranscriptStore({ path }))
            withContext(Dispatchers.Swing) { controller.start(settings) }
            val restored = controller.awaitState { !it.restoring && it.transcripts[chat.id]?.rows?.size == 3 }
            val reopened = restored.transcripts.getValue(chat.id)
            assertEquals("Final edit before closing", restored.drafts[chat.id])
            assertEquals(ContextUsage(96000, 128000), restored.sessions.single().contextUsage)
            assertEquals("retained.png", reopened.files.single().name)
            assertEquals("Retained thinking through the gap", reopened.rows[1].entry.content.single().text)
            assertEquals("Unfinished work", reopened.rows.last().entry.content.single().text)
            assertEquals(EntryPhase.Interrupted, reopened.rows.last().entry.phase)
            assertEquals("true", reopened.preferences["expanded:details:stream:b"])
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
                                    sendMessage(TranscriptSnapshot(chat.id, TranscriptCut("g", 0, entries = emptyList(), queue = queue)))
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
        val controller = TauController(Dispatchers.Swing, TranscriptStore({ directory.resolve("transcript.db").toString() }))
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
