package app.tau

import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.tooling.ComposeToolingApi
import androidx.compose.ui.ComposeDesktopEntryPoint
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.awt.ComposeWindow
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.WindowState
import androidx.compose.ui.window.awaitApplication
import io.ktor.server.application.install
import io.ktor.server.cio.CIO
import io.ktor.server.engine.embeddedServer
import io.ktor.server.routing.routing
import io.ktor.server.websocket.WebSockets
import io.ktor.server.websocket.webSocket
import io.ktor.websocket.Frame
import io.ktor.websocket.WebSocketSession
import io.ktor.websocket.readText
import io.ktor.websocket.send
import java.awt.GraphicsEnvironment
import java.awt.Robot
import java.awt.event.KeyEvent
import java.nio.file.Files
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.async
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.swing.Swing
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import org.junit.Assume.assumeFalse
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNull
import kotlin.test.assertTrue

@OptIn(ExperimentalComposeUiApi::class, ComposeToolingApi::class)
class DialogInputTest {
    @Test
    fun enterSubmitsSingleLineInputsAndKeepsEditorNewlines() = runBlocking {
        assumeFalse("Run with a display or Xvfb", GraphicsEnvironment.isHeadless())
        val root = Files.createTempDirectory("tau-dialog-input-").toFile()
        val controller = TauController(Dispatchers.Swing, LocalStore({ root.resolve("local.db").path }))
        val session = SessionSummary("chat", "Original title", SessionStatus.Idle, createdAtMs = 1, updatedAtMs = 1)
        val socket = CompletableDeferred<WebSocketSession>()
        val writes = Channel<ClientRequest>(Channel.UNLIMITED)
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                webSocket("/v1/ws") {
                    socket.complete(this)
                    send(TauJson.encodeToString<ServerMessage>(Hello(TauProtocolVersion, "fixture")))
                    send(TauJson.encodeToString<ServerMessage>(Sessions(listOf(session))))
                    for (frame in incoming) if (frame is Frame.Text) {
                        val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                        when (request) {
                            is OpenSession -> send(TauJson.encodeToString<ServerMessage>(TranscriptSnapshot(session.id,
                                TranscriptCut("g", 0, emptyList(), QueueState(), null))))
                            is GetTitlePrompt -> send(TauJson.encodeToString<ServerMessage>(TitlePrompt(request.id, "Title prompt", "Title prompt")))
                            is RenameSession, is RespondExtensionUi, is SetTitlePrompt -> writes.send(request)
                            else -> Unit
                        }
                        send(TauJson.encodeToString<ServerMessage>(Response(request.id, true)))
                    }
                }
            }
        }.start(wait = false)
        val settings = ConnectionSettings("http://127.0.0.1:${server.engine.resolvedConnectors().single().port}", "fixture")
        val visible = mutableStateOf(true)
        val window = CompletableDeferred<ComposeWindow>()
        val renderer = System.getProperty("skiko.renderApi")
        System.setProperty("skiko.renderApi", "SOFTWARE_COMPAT")
        withContext(Dispatchers.Swing) { controller.start(settings) }
        val application = async(Dispatchers.Default) {
            awaitApplication {
                if (visible.value) Window(onCloseRequest = { visible.value = false }, title = "Tau dialog input check",
                    state = WindowState(size = DpSize(1100.dp, 900.dp))) {
                    SideEffect { window.complete(this.window) }
                    TauApp(controller)
                } else exitApplication()
            }
        }
        suspend fun node(predicate: (SemanticsNode) -> Boolean): SemanticsNode = withTimeout(10_000) {
            var found: SemanticsNode? = null
            while (found == null) {
                found = withContext(Dispatchers.Swing) {
                    java.awt.Window.getWindows().filter { it.isShowing }.filterIsInstance<ComposeDesktopEntryPoint>()
                        .flatMap { it.semanticsOwners }.flatMap { owner ->
                            generateSequence(listOf(owner.rootSemanticsNode)) { level ->
                                level.flatMap { it.children }.takeIf { it.isNotEmpty() }
                            }.flatten().toList()
                        }.firstOrNull(predicate)
                }
                if (found == null) delay(30)
            }
            found
        }
        suspend fun click(label: String) {
            val button = node { it.config.getOrNull(SemanticsProperties.Text)?.any { text -> text.text == label } == true &&
                it.config.getOrNull(SemanticsActions.OnClick) != null }
            withContext(Dispatchers.Swing) { assertTrue(button.config[SemanticsActions.OnClick].action!!.invoke()) }
        }
        suspend fun edit(field: SemanticsNode, text: String) {
            withContext(Dispatchers.Swing) {
                assertTrue(field.config[SemanticsActions.RequestFocus].action!!.invoke())
                assertTrue(field.config[SemanticsActions.SetText].action!!.invoke(AnnotatedString(text)))
            }
            delay(150)
        }
        val robot = Robot()
        suspend fun enter() {
            robot.keyPress(KeyEvent.VK_ENTER)
            robot.keyRelease(KeyEvent.VK_ENTER)
            delay(200)
        }
        try {
            val frame = withTimeout(10_000) { window.await() }
            withContext(Dispatchers.Swing) { frame.toFront(); frame.requestFocus() }
            val card = node { it.config.getOrNull(SemanticsActions.OnLongClick) != null &&
                it.config.getOrNull(SemanticsProperties.Text)?.any { text -> text.text == session.title } == true }
            withContext(Dispatchers.Swing) { assertTrue(card.config[SemanticsActions.OnLongClick].action!!.invoke()) }
            click("Rename")
            val rename = node { it.config.getOrNull(SemanticsProperties.EditableText)?.text == session.title }
            edit(rename, "   ")
            enter()
            assertNull(withTimeoutOrNull(300) { writes.receive() }, "Enter submitted a blank title")
            edit(rename, "Renamed chat")
            enter()
            val renamed = assertIs<RenameSession>(withTimeoutOrNull(5_000) { writes.receive() }, "Enter did not save the chat title")
            assertEquals(session.id, renamed.sessionId)
            assertEquals("Renamed chat", renamed.title)
            click("Settings")
            withContext(Dispatchers.Swing) { controller.hideSettings() }
            for (value in listOf("Input value", "")) {
                val request = ExtensionUiRequest("input-$value", "input", prefill = "Input prefill")
                socket.await().send(TauJson.encodeToString<ServerMessage>(ExtensionUi(session.id, request)))
                val input = node { it.config.getOrNull(SemanticsProperties.EditableText)?.text == "Input prefill" }
                edit(input, value)
                if (value.isEmpty()) withContext(Dispatchers.Swing) {
                    assertEquals(ImeAction.Done, input.config[SemanticsProperties.ImeAction])
                    assertTrue(input.config[SemanticsActions.OnImeAction].action!!.invoke())
                } else enter()
                val response = assertIs<RespondExtensionUi>(withTimeout(5_000) { writes.receive() })
                assertEquals(request.id, response.requestId)
                assertEquals(value, response.value)
                assertEquals(false, response.cancelled)
            }
            socket.await().send(TauJson.encodeToString<ServerMessage>(ExtensionUi(session.id,
                ExtensionUiRequest("editor", "editor", prefill = "Editor text"))))
            val editor = node { it.config.getOrNull(SemanticsProperties.EditableText)?.text == "Editor text" }
            edit(editor, "Editor text")
            enter()
            node { it.config.getOrNull(SemanticsProperties.EditableText)?.text == "Editor text\n" }
            assertNull(withTimeoutOrNull(300) { writes.receive() }, "Enter submitted the multiline editor")
            click("Submit")
            val response = assertIs<RespondExtensionUi>(withTimeout(5_000) { writes.receive() })
            assertEquals("editor", response.requestId)
            assertEquals("Editor text\n", response.value)
            withContext(Dispatchers.Swing) { controller.showSettings() }
            val title = node { it.config.getOrNull(SemanticsProperties.EditableText)?.text == "Title prompt" &&
                it.config.getOrNull(SemanticsActions.RequestFocus) != null }
            edit(title, "Title prompt")
            enter()
            node { it.config.getOrNull(SemanticsProperties.EditableText)?.text == "Title prompt\n" }
            assertNull(withTimeoutOrNull(300) { writes.receive() }, "Enter saved the multiline title prompt")
            for (label in listOf("Daemon URL", "Access token")) {
                val field = node { it.config.getOrNull(SemanticsActions.SetText) != null &&
                    it.config.getOrNull(SemanticsProperties.Text)?.any { text -> text.text == label } == true }
                edit(field, "")
                enter()
                assertEquals("Enter an HTTP server URL and token.", controller.state.value.error, "$label did not submit connection validation")
                withContext(Dispatchers.Swing) { controller.dismissError() }
                edit(field, if (label == "Daemon URL") settings.serverUrl else settings.token)
            }
            enter()
            withTimeout(5_000) { while (controller.state.value.editingSettings) delay(20) }
            assertNull(withTimeoutOrNull(300) { writes.receive() }, "Keyboard submission sent a duplicate write")
        } finally {
            withContext(NonCancellable) {
                try {
                    withContext(Dispatchers.Swing) { visible.value = false; controller.dispose() }.join()
                    withTimeout(10_000) { application.await() }
                } finally {
                    if (renderer == null) System.clearProperty("skiko.renderApi") else System.setProperty("skiko.renderApi", renderer)
                    server.stop(0, 1000)
                    root.deleteRecursively()
                }
            }
        }
    }
}
