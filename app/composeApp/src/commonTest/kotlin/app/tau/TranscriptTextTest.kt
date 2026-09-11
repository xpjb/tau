package app.tau

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
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

            | Name | Value | Notes |
            |:---|:---:|---:|
            | **alpha** | 10 | [Tau](https://example.com) |
            | escaped\|pipe | `code` | Unicode κόσμε 中文 |
            | only first |
            | first | second | third | **KEEP-EXTRA** | escaped\|tail |

            Without edge pipes | Final
            --- | ---
            ordinary | LAST-CELL

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
        val visible = document.blocks.joinToString("\n") { block ->
            if (block.kind == TranscriptTextBlockKind.Table) block.rows.flatten().joinToString("\n") { it.text }
            else block.text.text
        }

        assertEquals(TranscriptTextBlockKind.Heading, document.blocks.first().kind)
        assertTrue("Heading" in visible)
        assertTrue("Paragraph with bold,  code , and Tau." in visible)
        assertTrue("• first\n  • nested\n☑ done" in visible)
        assertTrue("quoted" in visible)
        assertTrue("fn main() {\n    println!(\"hello\");\n}" in visible)
        assertTrue("Name" in visible && "Value" in visible && "alpha" in visible && "10" in visible)
        assertFalse("https://example.com" in visible)
        assertTrue(document.blocks.any { it.kind == TranscriptTextBlockKind.Code })
        val tables = document.blocks.filter { it.kind == TranscriptTextBlockKind.Table }
        assertEquals(2, tables.size)
        val table = tables.first()
        assertEquals(listOf(TextAlign.Left, TextAlign.Center, TextAlign.Right), table.alignments)
        assertEquals(listOf(
            listOf("Name", "Value", "Notes"),
            listOf("alpha", "10", "Tau"),
            listOf("escaped|pipe", "code", "Unicode κόσμε 中文"),
            listOf("only first"),
            listOf("first", "second", "third", "KEEP-EXTRA", "escaped|tail"),
        ), table.rows.map { row -> row.map { it.text.trim() } })
        assertTrue(table.rows[1][0].spanStyles.any { it.item.fontWeight == FontWeight.Bold })
        assertTrue(table.rows[4][3].spanStyles.any { it.item.fontWeight == FontWeight.Bold })
        assertTrue(table.rows[1][2].getLinkAnnotations(0, table.rows[1][2].length).isNotEmpty())
        assertEquals(listOf(listOf("Without edge pipes", "Final"), listOf("ordinary", "LAST-CELL")),
            tables.last().rows.map { row -> row.map { it.text.trim() } })
        assertTrue(document.blocks.any { it.kind == TranscriptTextBlockKind.Quote })
        assertTrue(document.blocks.any { it.kind == TranscriptTextBlockKind.Rule })
    }
}
