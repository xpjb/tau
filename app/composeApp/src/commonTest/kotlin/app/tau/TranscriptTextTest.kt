package app.tau

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class TranscriptTextTest {
    @Test
    fun convertsACompleteMarkdownMessageIntoRetainedTextBlocks() {
        val source = """
            # Heading

            Paragraph with **bold**, `code`, and [Tau](https://example.com).

            - first
              - nested
            - [x] done

            > quoted

            ```rust
            fn main() {
                println!("hello");
            }
            ```

            | Name | Value |
            |---|---:|
            | alpha | 10 |

            ---
        """.trimIndent()
        val document = buildChatText(
            text = source,
            markdown = true,
            styles = TranscriptTextStyles(
                body = TextStyle.Default,
                headings = List(6) { TextStyle.Default },
                code = TextStyle.Default,
                link = SpanStyle(color = Color.Blue),
                inlineCode = SpanStyle(background = Color.DarkGray),
                codeBackground = Color.Black,
                quoteBar = Color.Blue,
            ),
        )
        val visible = document.blocks.joinToString("\n") { it.text.text }

        assertEquals(TranscriptTextBlockKind.Heading, document.blocks.first().kind)
        assertTrue("Heading" in visible)
        assertTrue("Paragraph with bold,  code , and Tau." in visible)
        assertTrue("• first\n  • nested\n☑ done" in visible)
        assertTrue("quoted" in visible)
        assertTrue("fn main() {\n    println!(\"hello\");\n}" in visible)
        assertTrue("Name" in visible && "Value" in visible && "alpha" in visible && "10" in visible)
        assertFalse("https://example.com" in visible)
        assertTrue(document.blocks.any { it.kind == TranscriptTextBlockKind.Code })
        assertTrue(document.blocks.any { it.kind == TranscriptTextBlockKind.Table })
        assertTrue(document.blocks.any { it.kind == TranscriptTextBlockKind.Quote })
        assertTrue(document.blocks.any { it.kind == TranscriptTextBlockKind.Rule })
    }
}
