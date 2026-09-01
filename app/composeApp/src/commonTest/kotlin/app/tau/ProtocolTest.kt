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
        assertEquals(
            "{\"type\":\"delete_session\",\"id\":\"8\",\"sessionId\":\"chat\"}",
            TauJson.encodeToString<ClientRequest>(DeleteSession("8", "chat")),
        )

        val event = TauJson.decodeFromString<ServerMessage>(
            """{"type":"session_state","sessionId":"chat","status":"running","detail":"Running bash"}""",
        )
        val state = assertIs<SessionState>(event)
        assertEquals("chat", state.sessionId)
        assertEquals(SessionStatus.Running, state.status)
        assertEquals("Running bash", state.detail)

        val history = assertIs<History>(TauJson.decodeFromString<ServerMessage>(
            """{"type":"history","sessionId":"chat","messages":[{"entryId":"tool","role":"system","text":"Build","attachment":{"kind":"file","fileName":"tau.zip","caption":"Build"}}]}""",
        ))
        assertEquals(AttachmentKind.File, history.messages.single().attachment?.kind)
        assertEquals("tau.zip", history.messages.single().attachment?.fileName)
    }
}
