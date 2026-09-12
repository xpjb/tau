package app.tau

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.text.selection.SelectionState
import androidx.compose.foundation.text.selection.rememberSelectionState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.tooling.ComposeToolingApi
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.awt.ComposeWindow
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.WindowExceptionHandler
import java.awt.GraphicsEnvironment
import java.awt.Robot
import java.awt.Toolkit
import java.awt.datatransfer.DataFlavor
import java.awt.datatransfer.StringSelection
import java.awt.event.InputEvent
import java.awt.event.KeyEvent
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.swing.Swing
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.Assume.assumeFalse
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

@OptIn(ExperimentalComposeUiApi::class, ComposeToolingApi::class)
class TextSelectionTest {
    @Test
    fun selectsAndCopiesAcrossHorizontalViewportEdges() = runBlocking {
        assumeFalse("Run with a desktop-sized display or Xvfb", GraphicsEnvironment.isHeadless())
        val failure = CompletableDeferred<Throwable>()
        val code = "tool({\"command\":\"" + "sample_argument=12345; ".repeat(30) + "\"})"
        val scroll = ScrollState(0)
        val selection = CompletableDeferred<SelectionState>()
        val renderer = System.getProperty("skiko.renderApi")
        System.setProperty("skiko.renderApi", "SOFTWARE_COMPAT")
        val window = withContext(Dispatchers.Swing) {
            ComposeWindow().apply {
                setSize(1100, 760)
                setLocation(180, 100)
                exceptionHandler = WindowExceptionHandler { failure.complete(it); isVisible = false }
                setContent {
                    MaterialTheme {
                        val state = rememberSelectionState()
                        SideEffect { selection.complete(state) }
                        SelectionContainer(state, Modifier.fillMaxSize()) {
                            LazyColumn(reverseLayout = true) {
                                items((0 until 20).toList(), key = { it }) { index ->
                                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                        Text("Tool input $index")
                                        if (index == 0) Box(Modifier.width(650.dp).horizontalScroll(scroll)) {
                                            Text(code, fontFamily = FontFamily.Monospace, softWrap = false)
                                        } else Text("Earlier content $index")
                                        Text("End of tool $index")
                                    }
                                }
                            }
                        }
                    }
                }
                isVisible = true
                toFront()
                requestFocus()
            }
        }
        val robot = Robot()
        try {
            delay(600)
            for (direction in listOf(1, -1)) for (edge in listOf("center", "top", "bottom")) {
                withContext(Dispatchers.Swing) {
                    selection.await().clear()
                    scroll.scrollTo(if (direction > 0) 0 else scroll.maxValue)
                    Toolkit.getDefaultToolkit().systemClipboard.setContents(StringSelection("Unchanged clipboard"), null)
                }
                delay(200)
                val (start, target) = withContext(Dispatchers.Swing) {
                    val node = window.semanticsOwners.flatMap { owner ->
                        generateSequence(listOf(owner.rootSemanticsNode)) { level ->
                            level.flatMap { it.children }.takeIf { it.isNotEmpty() }
                        }.flatten().toList()
                    }.single { it.config.getOrNull(SemanticsProperties.Text)?.singleOrNull()?.text == code }
                    val layouts = mutableListOf<TextLayoutResult>()
                    assertTrue(node.config[SemanticsActions.GetTextLayoutResult].action!!.invoke(layouts))
                    val cursor = layouts.single().getCursorRect(if (direction > 0) 5 else code.length - 5)
                    val origin = node.positionOnScreen
                    val endX = origin.x + scroll.value + if (direction > 0) scroll.viewportSize + 130 else -130
                    val endY = origin.y + when (edge) { "top" -> 0f; "bottom" -> node.size.height.toFloat(); else -> cursor.center.y }
                    (origin + cursor.center) to androidx.compose.ui.geometry.Offset(endX, endY)
                }
                robot.mouseMove(start.x.toInt(), start.y.toInt())
                robot.mousePress(InputEvent.BUTTON1_DOWN_MASK)
                delay(60)
                for (step in 1..12) {
                    robot.mouseMove((start.x + (target.x - start.x) * step / 12).toInt(),
                        (start.y + (target.y - start.y) * step / 12).toInt())
                    delay(50)
                }
                val expected = if (direction > 0) code.drop(5) else code.dropLast(5)
                withTimeout(12_000) {
                    var step = 0
                    while (true) {
                        if (failure.isCompleted) throw failure.await()
                        robot.mouseMove(target.x.toInt() + step++ % 2, target.y.toInt())
                        delay(100)
                        if (withContext(Dispatchers.Swing) {
                            scroll.value == (if (direction > 0) scroll.maxValue else 0) &&
                                selection.await().selectedTexts.joinToString("") { it.text } == expected
                        }) break
                    }
                }
                robot.mouseRelease(InputEvent.BUTTON1_DOWN_MASK)
                robot.keyPress(KeyEvent.VK_CONTROL); robot.keyPress(KeyEvent.VK_C)
                robot.keyRelease(KeyEvent.VK_C); robot.keyRelease(KeyEvent.VK_CONTROL)
                val copied = withTimeout(5_000) {
                    var value = "Unchanged clipboard"
                    while (value == "Unchanged clipboard") {
                        delay(30)
                        value = withContext(Dispatchers.Swing) {
                            Toolkit.getDefaultToolkit().systemClipboard.getData(DataFlavor.stringFlavor) as String
                        }
                    }
                    value
                }
                assertEquals(expected, copied, "Copy changed at direction=$direction edge=$edge")
            }
        } finally {
            robot.mouseRelease(InputEvent.BUTTON1_DOWN_MASK)
            withContext(Dispatchers.Swing) { window.dispose() }
            if (renderer == null) System.clearProperty("skiko.renderApi") else System.setProperty("skiko.renderApi", renderer)
        }
    }
}
