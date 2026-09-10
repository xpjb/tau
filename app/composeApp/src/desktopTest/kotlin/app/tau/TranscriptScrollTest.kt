package app.tau

import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.tooling.ComposeToolingApi
import androidx.compose.ui.ComposeDesktopEntryPoint
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.awt.ComposeWindow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.WindowState
import androidx.compose.ui.window.awaitApplication
import io.ktor.server.application.install
import io.ktor.server.cio.CIO
import io.ktor.server.engine.embeddedServer
import io.ktor.http.ContentType
import io.ktor.server.response.respondBytes
import io.ktor.server.routing.get
import io.ktor.server.routing.routing
import io.ktor.server.websocket.WebSockets
import io.ktor.server.websocket.webSocket
import io.ktor.websocket.Frame
import io.ktor.websocket.readText
import io.ktor.websocket.send
import java.awt.GraphicsEnvironment
import java.awt.Robot
import java.awt.Rectangle
import java.awt.Toolkit
import java.awt.datatransfer.DataFlavor
import java.awt.datatransfer.StringSelection
import java.awt.event.InputEvent
import java.awt.event.KeyEvent
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import javax.imageio.ImageIO
import java.nio.file.Files
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.swing.Swing
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import org.junit.Assume.assumeFalse
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

@OptIn(ExperimentalComposeUiApi::class, ComposeToolingApi::class)
class TranscriptScrollTest {
    @Test
    fun scrollsCopiesAndZoomsImages() = runBlocking {
        assumeFalse("Run with a display or Xvfb", GraphicsEnvironment.isHeadless())
        checkScroll(grouped = false)
        checkScroll(grouped = true)
    }

    private suspend fun checkScroll(grouped: Boolean) = coroutineScope {
        val root = Files.createTempDirectory("tau-scroll-").toFile()
        val controller = TauController(Dispatchers.Swing, LocalStore({ root.resolve("local.db").path }))
        val session = SessionSummary("scroll", "Scroll check", SessionStatus.Idle, createdAtMs = 1, updatedAtMs = 1)
        val events = (0 until 1600).map { index ->
            TranscriptEvent("row:$index", index.toLong(), "entry-$index", role = if (!grouped && index % 2 == 0) EventRole.User else EventRole.Assistant,
                kind = if (grouped) EventKind.Thinking else EventKind.Text, text = if (!grouped && index % 2 == 0) "USER ROW $index" else (0 until 6).joinToString("\n\n") {
                    "[ROW $index paragraph $it](https://example.invalid/${"long-path-".repeat(30)})"
                })
        }
        val imageReads = AtomicInteger()
        val image = BufferedImage(1000, 700, BufferedImage.TYPE_INT_RGB)
        image.createGraphics().apply {
            color = java.awt.Color.CYAN; fillRect(0, 0, 1000, 700)
            color = java.awt.Color.RED; fillRect(630, 250, 40, 40)
            dispose()
        }
        val imageBytes = ByteArrayOutputStream().also { ImageIO.write(image, "png", it) }.toByteArray()
        val historyReads = AtomicInteger()
        val pageRequested = CompletableDeferred<Unit>()
        val releasePage = CompletableDeferred<Unit>()
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            install(WebSockets)
            routing {
                get("/v1/sessions/scroll/attachments/zoom") {
                    imageReads.incrementAndGet()
                    call.respondBytes(imageBytes, ContentType.Image.PNG)
                }
                webSocket("/v1/ws") {
                    send(TauJson.encodeToString<ServerMessage>(Hello(TauProtocolVersion, "fixture")))
                    send(TauJson.encodeToString<ServerMessage>(Sessions(listOf(session))))
                    for (frame in incoming) if (frame is Frame.Text) {
                        val request = TauJson.decodeFromString<ClientRequest>(frame.readText())
                        when (request) {
                            is OpenSession -> send(TauJson.encodeToString<ServerMessage>(TranscriptSnapshot(session.id,
                                TranscriptCut("g", 0, events.takeLast(50), QueueState(), 1550))))
                            is GetHistory -> {
                                if (historyReads.incrementAndGet() == 3) {
                                    pageRequested.complete(Unit)
                                    releasePage.await()
                                }
                                val page = events.filter { it.order < request.before }.takeLast(50)
                                send(TauJson.encodeToString<ServerMessage>(TranscriptPage(request.id, session.id, "g", request.before,
                                    HistoryPage(page, page.first().order.takeIf { it > 0 }))))
                            }
                            else -> assertTrue(request is ListSessions)
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
        if (grouped) controller.store.setPreference(ChatKey(settings.identity, session.id), "detailsDefault", "true")
        withContext(Dispatchers.Swing) { controller.start(settings) }
        val application = async(Dispatchers.Default) {
            awaitApplication {
                if (visible.value) Window(onCloseRequest = { visible.value = false }, title = "Tau scroll check",
                    state = WindowState(size = DpSize(1100.dp, 760.dp))) {
                    SideEffect { window.complete(this.window) }
                    TauApp(controller)
                } else exitApplication()
            }
        }
        suspend fun position(): ScrollPosition = withContext(Dispatchers.Swing) {
            TauJson.decodeFromString(controller.state.value.transcripts.getValue(session.id).preferences.getValue("scroll"))
        }
        suspend fun wheel(robot: Robot, direction: Int) {
            repeat(15) { robot.mouseWheel(direction); delay(15) }
            delay(180)
        }
        try {
            val frame = withTimeout(10_000) { window.await() }
            withTimeout(10_000) {
                while (controller.state.value.transcripts[session.id]?.rows?.size != 150) delay(20)
            }
            delay(500)
            val location = withContext(Dispatchers.Swing) { frame.toFront(); frame.requestFocus(); frame.locationOnScreen }
            val robot = Robot()
            robot.mouseMove(location.x + 740, location.y + 390)
            withTimeout(20_000) { while (!pageRequested.isCompleted) wheel(robot, -1) }
            delay(300)
            val anchor = position()
            assertTrue(!anchor.follow && anchor.key?.startsWith("row:") == true)
            releasePage.complete(Unit)
            withTimeout(10_000) { while (controller.state.value.transcripts.getValue(session.id).rows.size < 200) delay(20) }
            delay(500)
            assertEquals(anchor, position(), "Older history moved the reading position")
            var prior = anchor
            var reachedBottom = false
            repeat(40) {
                if (!reachedBottom) {
                    wheel(robot, 1)
                    val current = position()
                    if (current.follow) reachedBottom = true
                    else {
                        val before = checkNotNull(prior.key).substringAfter("row:").toInt()
                        val after = checkNotNull(current.key).substringAfter("row:").toInt()
                        assertTrue(after > before || after == before && current.offset <= prior.offset,
                            "Downward input moved backwards: $prior -> $current")
                    }
                    prior = current
                }
            }
            assertTrue(reachedBottom, "Downward scrolling never reached the newest message")
            assertEquals(3, historyReads.get(), "Downward scrolling fetched more older history")
            assertEquals("bottom-anchor", position().key)
            if (grouped) {
                val key = ChatKey(settings.identity, session.id)
                val copied = listOf(
                    TranscriptEvent("copy-first", 1600, "copy-first", role = EventRole.Assistant, kind = EventKind.Text, text = "First **reply**"),
                    TranscriptEvent("copy-thought", 1601, "copy-thought", role = EventRole.Assistant, kind = EventKind.Thinking, text = "Private thought"),
                    TranscriptEvent("copy-call", 1602, "copy-call", role = EventRole.Assistant, kind = EventKind.Tool, text = "Tool input", toolCallId = "copy-tool"),
                    TranscriptEvent("copy-output", 1603, "copy-output", role = EventRole.Tool, kind = EventKind.Text, text = "Tool output", toolCallId = "copy-tool"),
                    TranscriptEvent("copy-error", 1604, "copy-error", role = EventRole.Tool, kind = EventKind.Text, text = "Tool error", toolCallId = "copy-tool", isError = true),
                    TranscriptEvent("copy-last", 1605, "copy-last", role = EventRole.Assistant, kind = EventKind.Text, text = "Last reply"),
                    TranscriptEvent("copy-image", 1606, "copy-image", role = EventRole.Assistant, kind = EventKind.Image, text = "image/png"),
                )
                assertTrue(controller.store.applySnapshot(key, TranscriptCut("copy", 0, copied, QueueState(), null)))
                controller.store.setPreference(key, "expanded:tool:copy-call", "true")
                for (expanded in listOf(false, true)) {
                    controller.store.setPreference(key, "detailsDefault", expanded.toString())
                    delay(400)
                    withContext(Dispatchers.Swing) {
                        Toolkit.getDefaultToolkit().systemClipboard.setContents(StringSelection("Unchanged clipboard"), null)
                    }
                    robot.mouseMove(location.x + 700, location.y + 610)
                    delay(100)
                    robot.mousePress(InputEvent.BUTTON3_DOWN_MASK)
                    delay(80)
                    robot.mouseRelease(InputEvent.BUTTON3_DOWN_MASK)
                    delay(250)
                    robot.mouseMove(location.x + 750, location.y + 530)
                    delay(100)
                    robot.mousePress(InputEvent.BUTTON1_DOWN_MASK)
                    delay(80)
                    robot.mouseRelease(InputEvent.BUTTON1_DOWN_MASK)
                    val clipboard = withTimeout(5_000) {
                        var text = "Unchanged clipboard"
                        while (text == "Unchanged clipboard") {
                            delay(30)
                            text = withContext(Dispatchers.Swing) {
                                Toolkit.getDefaultToolkit().systemClipboard.getData(DataFlavor.stringFlavor) as String
                            }
                        }
                        text
                    }
                    assertEquals("First **reply**\n\nLast reply", clipboard, "Copy message included Details (expanded=$expanded)")
                }
                val photo = TranscriptEvent("zoom:0", 1700, "zoom", role = EventRole.Assistant, kind = EventKind.Image,
                    attachment = ChatAttachment(AttachmentKind.Image, "zoom.png", size = imageBytes.size.toLong()))
                assertTrue(controller.store.applySnapshot(key, TranscriptCut("image", 0, listOf(photo), QueueState(), null)))
                suspend fun clickImageControl(label: String) {
                    val point = withTimeout(5_000) {
                        var found: Offset? = null
                        while (found == null) {
                            found = withContext(Dispatchers.Swing) {
                                java.awt.Window.getWindows().filter { it.isShowing }.filterIsInstance<ComposeDesktopEntryPoint>()
                                    .flatMap { it.semanticsOwners }.asSequence().flatMap { owner ->
                                        generateSequence(listOf(owner.rootSemanticsNode)) { level ->
                                            level.flatMap { it.children }.takeIf { it.isNotEmpty() }
                                        }.flatten()
                                    }.firstOrNull { it.config.getOrNull(SemanticsProperties.ContentDescription)?.contains(label) == true }
                                    ?.let { it.positionOnScreen + Offset(it.size.width / 2f, it.size.height / 2f) }
                            }
                            if (found == null) delay(30)
                        }
                        found
                    }
                    robot.mouseMove(point.x.toInt(), point.y.toInt())
                    robot.mousePress(InputEvent.BUTTON1_DOWN_MASK)
                    delay(80)
                    robot.mouseRelease(InputEvent.BUTTON1_DOWN_MASK)
                    delay(350)
                }
                fun marker(): Rectangle {
                    val screen = robot.createScreenCapture(Rectangle(Toolkit.getDefaultToolkit().screenSize))
                    var left = screen.width; var top = screen.height; var right = -1; var bottom = -1
                    for (y in 0 until screen.height) for (x in 0 until screen.width) {
                        val pixel = screen.getRGB(x, y)
                        if (pixel shr 16 and 255 > 240 && pixel shr 8 and 255 < 32 && pixel and 255 < 32) {
                            left = minOf(left, x); right = maxOf(right, x)
                            top = minOf(top, y); bottom = maxOf(bottom, y)
                        }
                    }
                    assertTrue(right >= left && bottom >= top, "Image marker is missing")
                    return Rectangle(left, top, right - left + 1, bottom - top + 1)
                }
                clickImageControl("zoom.png")
                val fitted = marker()
                val x = fitted.centerX.toInt(); val y = fitted.centerY.toInt()
                robot.mouseMove(x, y)
                robot.mouseWheel(-4)
                delay(400)
                val enlarged = marker()
                assertTrue(enlarged.width > fitted.width * 1.8, "Wheel input did not zoom the full-screen image: $fitted -> $enlarged")
                assertTrue(kotlin.math.abs(enlarged.centerX - fitted.centerX) < 3 && kotlin.math.abs(enlarged.centerY - fitted.centerY) < 3,
                    "Wheel zoom moved the image point under the cursor")
                robot.mouseMove(x + 20, y + 20)
                delay(500)
                assertEquals(enlarged, marker(), "Hover moved or reset persistent zoom")
                robot.mousePress(InputEvent.BUTTON1_DOWN_MASK)
                delay(80)
                robot.mouseMove(x + 70, y + 60)
                delay(100)
                robot.mouseRelease(InputEvent.BUTTON1_DOWN_MASK)
                delay(350)
                val panned = marker()
                assertEquals(enlarged.width, panned.width)
                assertTrue(kotlin.math.abs(panned.x - enlarged.x - 50) < 3 && kotlin.math.abs(panned.y - enlarged.y - 40) < 3,
                    "Dragging did not pan the zoomed image: $enlarged -> $panned")
                robot.mousePress(InputEvent.BUTTON1_DOWN_MASK)
                robot.mouseRelease(InputEvent.BUTTON1_DOWN_MASK)
                delay(250)
                assertEquals(panned, marker(), "A tap dismissed or changed the image")
                clickImageControl("Fit image")
                assertEquals(fitted, marker())
                clickImageControl("Zoom in")
                assertTrue(marker().width > fitted.width * 1.8)
                clickImageControl("Zoom out")
                assertEquals(fitted, marker())
                clickImageControl("Zoom in")
                clickImageControl("Close image")
                clickImageControl("zoom.png")
                assertEquals(fitted, marker(), "Reopening retained old zoom")
                robot.keyPress(KeyEvent.VK_ESCAPE); robot.keyRelease(KeyEvent.VK_ESCAPE)
                delay(300)
                clickImageControl("zoom.png")
                assertEquals(fitted, marker())
                clickImageControl("Close image")
                assertEquals(1, imageReads.get(), "Zooming downloaded the image again")
            }
        } finally {
            withContext(NonCancellable) {
                releasePage.complete(Unit)
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
