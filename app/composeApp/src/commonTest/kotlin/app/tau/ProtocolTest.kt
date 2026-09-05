package app.tau

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlinx.serialization.encodeToString

class ProtocolTest {
    @Test
    fun fuzzy_completion_matches_compact_model_queries() {
        val direct = fuzzyCompletionScore("openai-codex/gpt-5.6-sol", "cod sol")
        val scattered = fuzzyCompletionScore("test/c---o---d---s---o---l", "cod sol")
        assertNotNull(direct)
        assertNotNull(scattered)
        kotlin.test.assertTrue(direct < scattered)
        assertNull(fuzzyCompletionScore("anthropic/claude-sonnet", "gpt sol"))
    }

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

        val sessions = assertIs<Sessions>(TauJson.decodeFromString<ServerMessage>(
            """{"type":"sessions","sessions":[{"id":"chat","title":"Work","status":"idle","model":{"provider":"openai-codex","modelId":"gpt-5.6-sol"},"createdAtMs":1,"updatedAtMs":2}]}""",
        ))
        assertEquals("openai-codex", sessions.sessions.single().model?.provider)
        assertEquals("gpt-5.6-sol", sessions.sessions.single().model?.modelId)

        val history = assertIs<History>(TauJson.decodeFromString<ServerMessage>(
            """{"type":"history","sessionId":"chat","messages":[{"entryId":"tool","role":"system","text":"Build","attachment":{"kind":"file","fileName":"tau.zip","caption":"Build","size":12345}}]}""",
        ))
        assertEquals(AttachmentKind.File, history.messages.single().attachment?.kind)
        assertEquals("tau.zip", history.messages.single().attachment?.fileName)
        assertEquals(12345L, history.messages.single().attachment?.size)

        val detailedHistory = assertIs<History>(TauJson.decodeFromString<ServerMessage>(
            """{"type":"history","sessionId":"chat","messages":[{"entryId":"answer","role":"assistant","text":"Done","hasDetails":true,"details":[{"kind":"thinking","text":"Checking"},{"kind":"tool","toolName":"bash","arguments":"cargo check","result":"Finished","isError":false}]}]}""",
        ))
        assertEquals(true, detailedHistory.messages.single().hasDetails)
        assertEquals("Checking", detailedHistory.messages.single().details[0].text)
        assertEquals("bash", detailedHistory.messages.single().details[1].toolName)
        assertEquals("Finished", detailedHistory.messages.single().details[1].result)

        val detailsDelta = assertIs<StreamDetailsDelta>(
            TauJson.decodeFromString<ServerMessage>(
                """{"type":"stream_details_delta","sessionId":"chat","delta":"Planning"}""",
            ),
        )
        assertEquals("Planning", detailsDelta.delta)

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
