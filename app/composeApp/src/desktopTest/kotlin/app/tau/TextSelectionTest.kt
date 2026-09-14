package app.tau

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
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
        val multiline = "a".repeat(67) + "\n" + "b".repeat(485)
        var sample by mutableStateOf("line")
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
                                        if (index == 0) Box(Modifier.width(650.dp)
                                            .then(if (sample == "wrapped") Modifier else Modifier.horizontalScroll(scroll))) {
                                            Text(if (sample == "line") code else multiline,
                                                if (sample == "line") Modifier.height(40.dp) else Modifier,
                                                fontFamily = FontFamily.Monospace, softWrap = sample == "wrapped")
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
            for (kind in listOf("multiline", "wrapped", "line", "widgets")) {
                withContext(Dispatchers.Swing) { selection.await().clear(); sample = if (kind == "widgets") "wrapped" else kind }
                delay(200)
                val text = if (kind == "line") code else multiline
                for (direction in listOf(1, -1)) for (edge in if (kind == "line") listOf("center", "top", "bottom") else listOf("side")) {
                    println("Selecting sample=$kind direction=$direction edge=$edge")
                    withContext(Dispatchers.Swing) {
                        selection.await().clear()
                        scroll.scrollTo(if (direction > 0) 0 else scroll.maxValue)
                        Toolkit.getDefaultToolkit().systemClipboard.setContents(StringSelection("Unchanged clipboard"), null)
                    }
                    delay(200)
                    val (start, target, expected) = withContext(Dispatchers.Swing) {
                        val nodes = window.semanticsOwners.flatMap { owner ->
                            generateSequence(listOf(owner.rootSemanticsNode)) { level ->
                                level.flatMap { it.children }.takeIf { it.isNotEmpty() }
                            }.flatten().toList()
                        }
                        val node = nodes.single { it.config.getOrNull(SemanticsProperties.Text)?.singleOrNull()?.text == text }
                        val layouts = mutableListOf<TextLayoutResult>()
                        assertTrue(node.config[SemanticsActions.GetTextLayoutResult].action!!.invoke(layouts))
                        val layout = layouts.single()
                        val startOffset = if (kind == "widgets") text.length - 1 else if (direction > 0) { if (kind == "line") 5 else 1 }
                            else text.length - if (kind == "line") 5 else 1
                        val cursor = layout.getCursorRect(startOffset)
                        val origin = node.positionOnScreen
                        var start = origin + cursor.center
                        val target: androidx.compose.ui.geometry.Offset
                        val expected: String
                        if (kind == "line") {
                            assertTrue(layout.size.height > layout.multiParagraph.height, "Exercise padding beyond the glyphs")
                            val endX = origin.x + scroll.value + if (direction > 0) scroll.viewportSize + 130 else -130
                            val endY = origin.y + when (edge) { "top" -> 0f; "bottom" -> node.size.height.toFloat(); else -> cursor.center.y }
                            target = androidx.compose.ui.geometry.Offset(endX, endY)
                            expected = if (direction > 0) text.drop(5) else text.dropLast(5)
                        } else if (kind == "widgets") {
                            val footer = "End of tool 0"
                            val footerNode = nodes.single { it.config.getOrNull(SemanticsProperties.Text)?.singleOrNull()?.text == footer }
                            val footerLayouts = mutableListOf<TextLayoutResult>()
                            assertTrue(footerNode.config[SemanticsActions.GetTextLayoutResult].action!!.invoke(footerLayouts))
                            val footerPoint = footerNode.positionOnScreen + footerLayouts.single().getCursorRect(5).center
                            target = if (direction > 0) footerPoint else start
                            if (direction < 0) start = footerPoint
                            expected = text.takeLast(1) + "\n" + footer.take(5)
                        } else {
                            val endOffset = if (direction > 0) layout.getLineStart(1) else 0
                            val localTarget = androidx.compose.ui.geometry.Offset(
                                if (direction > 0) -8f else node.size.width + 8f,
                                layout.getCursorRect(endOffset).center.y)
                            target = origin + localTarget
                            val selectedEnd = layout.getOffsetForPosition(localTarget)
                            expected = text.substring(minOf(startOffset, selectedEnd), maxOf(startOffset, selectedEnd))
                            if (kind == "multiline" && direction > 0) assertEquals(68, selectedEnd)
                        }
                        Triple(start, target, expected)
                    }
                    robot.mouseMove(start.x.toInt(), start.y.toInt())
                    robot.mousePress(InputEvent.BUTTON1_DOWN_MASK)
                    delay(60)
                    for (step in 1..12) {
                        robot.mouseMove((start.x + (target.x - start.x) * step / 12).toInt(),
                            (start.y + (target.y - start.y) * step / 12).toInt())
                        delay(50)
                    }
                    withTimeout(12_000) {
                        var step = 0
                        while (true) {
                            if (failure.isCompleted) throw failure.await()
                            robot.mouseMove(target.x.toInt() + step++ % 2, target.y.toInt())
                            delay(100)
                            if (withContext(Dispatchers.Swing) {
                                (kind != "line" || scroll.value == (if (direction > 0) scroll.maxValue else 0)) &&
                                    selection.await().selectedTexts.joinToString("\n") { it.text } == expected
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
                    assertEquals(expected, copied, "Copy changed at sample=$kind direction=$direction edge=$edge")
                }
            }
        } finally {
            robot.mouseRelease(InputEvent.BUTTON1_DOWN_MASK)
            withContext(Dispatchers.Swing) { window.dispose() }
            if (renderer == null) System.clearProperty("skiko.renderApi") else System.setProperty("skiko.renderApi", renderer)
        }
    }
}
