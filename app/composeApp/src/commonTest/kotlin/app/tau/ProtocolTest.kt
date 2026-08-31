package app.tau

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlinx.serialization.encodeToString

class ProtocolTest {
    @Test
    fun encodes_commands_and_decodes_daemon_events() {
        assertEquals(
            "{\"type\":\"fork_session\",\"id\":\"7\",\"sessionId\":\"chat\",\"entryId\":\"entry\"}",
            TauJson.encodeToString<ClientRequest>(ForkSession("7", "chat", "entry")),
        )

        val event = TauJson.decodeFromString<ServerMessage>(
            """{"type":"session_state","sessionId":"chat","status":"running","detail":"Running bash"}""",
        )
        val state = assertIs<SessionState>(event)
        assertEquals("chat", state.sessionId)
        assertEquals(SessionStatus.Running, state.status)
        assertEquals("Running bash", state.detail)
    }
}
