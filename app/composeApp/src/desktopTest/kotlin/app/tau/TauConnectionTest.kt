package app.tau

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
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.swing.Swing
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertTrue

class TauConnectionTest {
    private val chat = SessionSummary(
        id = "chat",
        title = "Test chat",
        status = SessionStatus.Idle,
        createdAtMs = 1,
        updatedAtMs = 1,
    )

    private suspend fun WebSocketSession.sendMessage(message: ServerMessage) {
        send(TauJson.encodeToString<ServerMessage>(message))
    }

    private suspend fun TauController.awaitState(predicate: (TauUiState) -> Boolean): TauUiState =
        withTimeout(10_000) { state.first(predicate) }

    private suspend fun Channel<ClientRequest>.nextRequest(): ClientRequest =
        withTimeout(10_000) { receive() }

    @Test
    fun keeps_sends_visible_and_refreshes_after_cancel_and_reconnect() = runBlocking {
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
                    for (frame in incoming) {
                        if (frame is Frame.Text) {
                            requests.send(TauJson.decodeFromString<ClientRequest>(frame.readText()))
                        }
                    }
                }
            }
        }.start(wait = false)
        val port = server.engine.resolvedConnectors().single().port
        val controller = TauController(Dispatchers.Swing)
        try {
            withContext(Dispatchers.Swing) {
                controller.start(ConnectionSettings("http://127.0.0.1:$port", "test-token"))
            }
            var socket = withTimeout(10_000) { sockets.receive() }
            var open = assertIs<OpenSession>(requests.nextRequest())
            socket.sendMessage(History(chat.id, emptyList()))
            socket.sendMessage(StreamSnapshot(chat.id, emptyList()))
            socket.sendMessage(Response(open.id, true, chat.id))
            controller.awaitState { chat.id in it.histories }

            withContext(Dispatchers.Swing) {
                controller.setDraft(chat.id, "Repeat")
                controller.sendPrompt()
            }
            val first = assertIs<Prompt>(requests.nextRequest())
            val sending = controller.awaitState {
                it.outgoingMessages[chat.id]?.singleOrNull()?.status == OutgoingStatus.Sending &&
                    chat.id !in it.uploadingSessions
            }
            assertEquals("", sending.drafts[chat.id])
            socket.sendMessage(Response(first.id, true, chat.id))
            controller.awaitState {
                it.outgoingMessages[chat.id]?.singleOrNull()?.status == OutgoingStatus.Waiting
            }

            withContext(Dispatchers.Swing) {
                controller.setDraft(chat.id, "Repeat")
                controller.sendPrompt()
            }
            val second = assertIs<Prompt>(requests.nextRequest())
            val repeated = controller.awaitState { it.outgoingMessages[chat.id]?.size == 2 }
            assertEquals(listOf(0, 1), repeated.outgoingMessages.getValue(chat.id)
                .map(OutgoingMessage::canonicalOccurrence))
            val history = listOf(
                ChatMessage("u1", ChatRole.User, "Repeat", timestampMs = 1),
                ChatMessage("u2", ChatRole.User, "Repeat", timestampMs = 2),
            )
            socket.sendMessage(History(chat.id, history.take(1)))
            controller.awaitState {
                it.outgoingMessages[chat.id]?.singleOrNull()?.requestId == second.id
            }
            socket.sendMessage(Response(second.id, true, chat.id))
            socket.sendMessage(History(chat.id, history))
            controller.awaitState { it.outgoingMessages[chat.id].isNullOrEmpty() }

            withContext(Dispatchers.Swing) {
                controller.setDraft(chat.id, "Rejected")
                controller.sendPrompt()
            }
            val rejected = assertIs<Prompt>(requests.nextRequest())
            controller.awaitState { it.outgoingMessages[chat.id]?.size == 1 }
            socket.sendMessage(Response(rejected.id, false, error = "Test rejection"))
            controller.awaitState {
                it.outgoingMessages[chat.id].isNullOrEmpty() && it.drafts[chat.id] == "Rejected"
            }

            withContext(Dispatchers.Swing) {
                controller.setDraft(chat.id, "/thinking high")
                controller.sendPrompt()
            }
            val commands = assertIs<GetCommands>(requests.nextRequest())
            socket.sendMessage(Commands(chat.id, emptyList()))
            socket.sendMessage(Response(commands.id, true, chat.id))
            val command = assertIs<Prompt>(requests.nextRequest())
            socket.sendMessage(Response(command.id, true, chat.id, commandHandled = true, notice = "Done"))
            controller.awaitState {
                it.outgoingMessages[chat.id].isNullOrEmpty() && it.notice == "Done"
            }

            socket.sendMessage(StreamReset(chat.id, "old-live", 3))
            socket.sendMessage(StreamDelta(chat.id, "old-live", 0, "Working"))
            socket.sendMessage(SessionState(chat.id, SessionStatus.Running))
            controller.awaitState {
                it.liveAttempts[chat.id]?.singleOrNull()?.content?.singleOrNull()?.text == "Working" &&
                    it.sessions.single().status == SessionStatus.Running
            }
            withContext(Dispatchers.Swing) { controller.abort() }
            val abort = assertIs<Abort>(requests.nextRequest())
            socket.sendMessage(Response(abort.id, true, chat.id))
            socket.sendMessage(SessionState(chat.id, SessionStatus.Idle))
            socket.sendMessage(StreamSnapshot(chat.id, emptyList()))
            controller.awaitState {
                it.liveAttempts[chat.id].isNullOrEmpty() && it.sessions.single().status == SessionStatus.Idle
            }

            socket.sendMessage(StreamReset(chat.id, "lost-live", 4))
            socket.sendMessage(StreamDelta(chat.id, "lost-live", 0, "Old output"))
            controller.awaitState { it.liveAttempts[chat.id]?.size == 1 }
            withContext(Dispatchers.Swing) {
                controller.setDraft(chat.id, "Applied without acknowledgement")
                controller.sendPrompt()
            }
            val unacknowledged = assertIs<Prompt>(requests.nextRequest())
            controller.awaitState {
                it.outgoingMessages[chat.id]?.singleOrNull()?.status == OutgoingStatus.Sending
            }
            socket.close(CloseReason(CloseReason.Codes.GOING_AWAY, "Reconnect test"))
            controller.awaitState {
                it.connectionStatus == ConnectionStatus.Offline &&
                    it.liveAttempts.isEmpty() &&
                    it.outgoingMessages[chat.id]?.singleOrNull()?.status == OutgoingStatus.Unconfirmed
            }
            socket = withTimeout(10_000) { sockets.receive() }
            open = assertIs<OpenSession>(requests.nextRequest())
            socket.sendMessage(History(chat.id, history))
            socket.sendMessage(StreamSnapshot(chat.id, emptyList()))
            socket.sendMessage(SessionState(chat.id, SessionStatus.Sleeping))
            socket.sendMessage(Response(open.id, true, chat.id))
            val restored = controller.awaitState { it.sessions.single().status == SessionStatus.Sleeping }
            assertTrue(restored.liveAttempts.isEmpty())
            assertEquals(OutgoingStatus.Unconfirmed, restored.outgoingMessages.getValue(chat.id).single().status)
            socket.sendMessage(History(chat.id, history + ChatMessage(
                "u3", ChatRole.User, unacknowledged.text, timestampMs = 5,
            )))
            controller.awaitState {
                it.histories[chat.id]?.size == 3 && it.outgoingMessages[chat.id].isNullOrEmpty()
            }
            assertTrue(requests.tryReceive().isFailure, "Reconnect must not resend prompts")
        } finally {
            withContext(Dispatchers.Swing) { controller.dispose() }
            server.stop(0, 1_000)
            sockets.close()
            requests.close()
        }
    }

    @Test
    fun reconnects_when_commands_arrive_but_the_return_path_stops() = runBlocking {
        val connections = AtomicInteger()
        val applied = AtomicBoolean()
        val requests = Channel<ClientRequest>(Channel.UNLIMITED)
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                webSocketRaw("/v1/ws") {
                    val connection = connections.incrementAndGet()
                    sendMessage(Hello(TauProtocolVersion, "test-$connection"))
                    sendMessage(Sessions(if (applied.get()) listOf(chat) else emptyList()))
                    for (frame in incoming) {
                        when (frame) {
                            is Frame.Ping -> if (connection > 1) send(Frame.Pong(frame.data))
                            is Frame.Text -> {
                                val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                                requests.send(request)
                                when (request) {
                                    is CreateSession -> applied.set(true)
                                    is OpenSession -> {
                                        sendMessage(History(chat.id, emptyList()))
                                        sendMessage(StreamSnapshot(chat.id, emptyList()))
                                        sendMessage(Response(request.id, true, chat.id))
                                    }
                                    else -> Unit
                                }
                            }
                            else -> Unit
                        }
                    }
                }
            }
        }.start(wait = false)
        val port = server.engine.resolvedConnectors().single().port
        val controller = TauController(Dispatchers.Swing)
        try {
            withContext(Dispatchers.Swing) {
                controller.start(ConnectionSettings("http://127.0.0.1:$port", "test-token"))
            }
            controller.awaitState { it.connectionStatus == ConnectionStatus.Connected }
            withContext(Dispatchers.Swing) { controller.createSession() }
            assertIs<CreateSession>(requests.nextRequest())
            withTimeout(60_000) {
                controller.state.first {
                    it.connectionStatus == ConnectionStatus.Connected &&
                        it.daemonVersion != "test-1" && chat.id in it.histories
                }
            }
            assertTrue(applied.get())
            assertTrue(connections.get() >= 2)
            assertIs<OpenSession>(requests.nextRequest())
            assertTrue(requests.tryReceive().isFailure, "Reconnect must not repeat the applied command")
        } finally {
            withContext(Dispatchers.Swing) { controller.dispose() }
            server.stop(0, 1_000)
            requests.close()
        }
    }
}
