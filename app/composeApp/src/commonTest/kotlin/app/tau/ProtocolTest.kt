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
        assertEquals(
            "{\"type\":\"get_commands\",\"id\":\"9\",\"sessionId\":\"chat\"}",
            TauJson.encodeToString<ClientRequest>(GetCommands("9", "chat")),
        )
        assertEquals(
            "{\"type\":\"extension_ui_response\",\"id\":\"10\",\"sessionId\":\"chat\",\"requestId\":\"dialog\",\"value\":\"One\"}",
            TauJson.encodeToString<ClientRequest>(
                RespondExtensionUi("10", "chat", "dialog", value = "One"),
            ),
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

        val commands = assertIs<Commands>(TauJson.decodeFromString<ServerMessage>(
            """{"type":"commands","sessionId":"chat","commands":[{"name":"model","description":"Select model","source":"builtin","argumentHint":"<provider/model>","arguments":[{"value":"test/model","description":"Test"}]}]}""",
        ))
        assertEquals("model", commands.commands.single().name)
        assertEquals("test/model", commands.commands.single().arguments.single().value)

        val extensionUi = assertIs<ExtensionUi>(TauJson.decodeFromString<ServerMessage>(
            """{"type":"extension_ui","sessionId":"chat","request":{"id":"dialog","method":"select","title":"Choose","options":["One","Two"]}}""",
        ))
        assertEquals(listOf("One", "Two"), extensionUi.request.options)

        val response = assertIs<Response>(TauJson.decodeFromString<ServerMessage>(
            """{"type":"response","requestId":"11","ok":true,"commandHandled":true,"notice":"Done"}""",
        ))
        assertEquals(true, response.commandHandled)
        assertEquals("Done", response.notice)
    }
}
