package app.tau

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.mikepenz.markdown.annotator.DefaultAnnotatorSettings
import com.mikepenz.markdown.annotator.buildMarkdownAnnotatedString
import com.mikepenz.markdown.model.State
import com.mikepenz.markdown.model.markdownAnnotator
import com.mikepenz.markdown.model.parseMarkdown
import org.intellij.markdown.IElementType
import org.intellij.markdown.MarkdownElementTypes
import org.intellij.markdown.MarkdownTokenTypes
import org.intellij.markdown.ast.ASTNode
import org.intellij.markdown.ast.findChildOfType
import org.intellij.markdown.ast.getTextInNode
import org.intellij.markdown.flavours.gfm.GFMElementTypes
import org.intellij.markdown.flavours.gfm.GFMFlavourDescriptor
import org.intellij.markdown.flavours.gfm.GFMTokenTypes
import org.intellij.markdown.parser.CancellationToken
import org.intellij.markdown.parser.MarkdownParser

internal data class TranscriptTextStyles(
    val body: TextStyle,
    val headings: List<TextStyle>,
    val code: TextStyle,
    val link: SpanStyle,
    val inlineCode: SpanStyle,
    val codeBackground: Color,
    val quoteBar: Color,
    val blockSpacing: Dp = 8.dp,
    val codePadding: Dp = 12.dp,
    val quoteIndent: Dp = 10.dp,
    val quoteBarWidth: Dp = 2.dp,
)

internal enum class TranscriptTextBlockKind {
    Flow,
    Heading,
    Code,
    Quote,
    Rule,
}

internal data class TranscriptTextBlock(
    val text: AnnotatedString,
    val style: TextStyle,
    val kind: TranscriptTextBlockKind,
)

internal data class MeasuredTranscriptTextBlock(
    private val source: TranscriptTextBlock,
    val height: Int,
) {
    val text: AnnotatedString
        get() = source.text
    val style: TextStyle
        get() = source.style
    val kind: TranscriptTextBlockKind
        get() = source.kind
}

internal data class TranscriptTextDocument(
    val blocks: List<TranscriptTextBlock>,
)

internal data class MeasuredTranscriptText(
    val blocks: List<MeasuredTranscriptTextBlock>,
    val height: Int,
)

private val MarkdownSyntax = Regex(
    """(?m)(https?://|[*_`\[\]#>|~<]|^\s*(?:(?:-{3,})\s*$|[-+] |\d+[.)] ))""",
)
private val ChatMarkdownFlavour = GFMFlavourDescriptor()
private val ChatMarkdownParser = MarkdownParser(
    ChatMarkdownFlavour,
    false,
    CancellationToken.NonCancellable,
)

internal fun buildChatText(
    text: String,
    markdown: Boolean,
    styles: TranscriptTextStyles,
): TranscriptTextDocument {
    val blocks = if (!markdown || !MarkdownSyntax.containsMatchIn(text)) {
        listOf(
            TranscriptTextBlock(
                text = AnnotatedString(text),
                style = styles.body,
                kind = TranscriptTextBlockKind.Flow,
            ),
        )
    } else {
        val parsed = parseMarkdown(
            content = text,
            flavour = ChatMarkdownFlavour,
            parser = ChatMarkdownParser,
        )
        if (parsed !is State.Success) {
            listOf(
                TranscriptTextBlock(
                    text = AnnotatedString(text),
                    style = styles.body,
                    kind = TranscriptTextBlockKind.Flow,
                ),
            )
        } else {
            val annotatorSettings = DefaultAnnotatorSettings(
                linkTextSpanStyle = TextLinkStyles(style = styles.link),
                codeSpanStyle = styles.inlineCode,
                annotator = markdownAnnotator(),
                referenceLinkHandler = parsed.referenceLinkHandler,
                linkInteractionListener = null,
            )
            val result = mutableListOf<TranscriptTextBlock>()

            fun annotated(node: ASTNode, childType: IElementType? = null): AnnotatedString {
                val contentNode = childType?.let(node::findChildOfType) ?: node
                return buildAnnotatedString {
                    buildMarkdownAnnotatedString(text, contentNode, annotatorSettings)
                }
            }

            fun flattened(node: ASTNode): AnnotatedString = buildAnnotatedString {
                fun appendNode(current: ASTNode, depth: Int) {
                    when (current.type) {
                        MarkdownElementTypes.PARAGRAPH -> append(annotated(current))
                        MarkdownElementTypes.ORDERED_LIST,
                        MarkdownElementTypes.UNORDERED_LIST -> {
                            var item = 0
                            current.children.filter { it.type == MarkdownElementTypes.LIST_ITEM }
                                .forEach { child ->
                                    if (length > 0) append('\n')
                                    repeat(depth) { append("  ") }
                                    val checkbox = child.children.firstOrNull {
                                        it.type == GFMTokenTypes.CHECK_BOX
                                    }
                                    if (checkbox == null) {
                                        append(
                                            if (current.type == MarkdownElementTypes.ORDERED_LIST) {
                                                "${++item}. "
                                            } else {
                                                "• "
                                            },
                                        )
                                    }
                                    appendNode(child, depth + 1)
                                }
                        }
                        MarkdownElementTypes.LIST_ITEM -> {
                            var hasContent = false
                            current.children.forEach { child ->
                                when (child.type) {
                                    MarkdownTokenTypes.LIST_BULLET,
                                    MarkdownTokenTypes.LIST_NUMBER,
                                    MarkdownTokenTypes.WHITE_SPACE,
                                    MarkdownTokenTypes.EOL -> Unit
                                    GFMTokenTypes.CHECK_BOX -> {
                                        val marker = child.getTextInNode(text).toString()
                                        append(if ('x' in marker.lowercase()) "☑ " else "☐ ")
                                    }
                                    else -> {
                                        if (
                                            hasContent &&
                                            length > 0 &&
                                            child.type != MarkdownElementTypes.ORDERED_LIST &&
                                            child.type != MarkdownElementTypes.UNORDERED_LIST
                                        ) {
                                            append('\n')
                                        }
                                        appendNode(child, depth)
                                        hasContent = true
                                    }
                                }
                            }
                        }
                        MarkdownElementTypes.BLOCK_QUOTE,
                        GFMElementTypes.ALERT -> current.children.forEachIndexed { index, child ->
                            if (index > 0 && length > 0) append('\n')
                            appendNode(child, depth)
                        }
                        GFMElementTypes.TABLE -> current.children
                            .filter { it.type == GFMElementTypes.HEADER || it.type == GFMElementTypes.ROW }
                            .forEachIndexed { rowIndex, row ->
                                if (rowIndex > 0) append('\n')
                                val cells = row.children.filter { it.type == GFMTokenTypes.CELL }
                                cells.forEachIndexed { cellIndex, cell ->
                                    if (cellIndex > 0) append("  │  ")
                                    append(annotated(cell))
                                }
                            }
                        MarkdownElementTypes.IMAGE -> {
                            val label = current.findChildOfType(MarkdownElementTypes.LINK_TEXT)
                            append(if (label == null) "[image]" else annotated(label))
                        }
                        MarkdownElementTypes.LINK_DEFINITION,
                        MarkdownTokenTypes.BLOCK_QUOTE,
                        MarkdownTokenTypes.EOL,
                        MarkdownTokenTypes.WHITE_SPACE -> Unit
                        else -> {
                            if (current.children.isEmpty()) {
                                append(current.getTextInNode(text))
                            } else {
                                current.children.forEach { appendNode(it, depth) }
                            }
                        }
                    }
                }
                appendNode(node, 0)
            }

            parsed.node.children.forEach { node ->
                when (node.type) {
                    MarkdownElementTypes.PARAGRAPH -> result += TranscriptTextBlock(
                        annotated(node),
                        styles.body,
                        TranscriptTextBlockKind.Flow,
                    )
                    MarkdownElementTypes.ATX_1,
                    MarkdownElementTypes.ATX_2,
                    MarkdownElementTypes.ATX_3,
                    MarkdownElementTypes.ATX_4,
                    MarkdownElementTypes.ATX_5,
                    MarkdownElementTypes.ATX_6 -> {
                        val level = when (node.type) {
                            MarkdownElementTypes.ATX_1 -> 0
                            MarkdownElementTypes.ATX_2 -> 1
                            MarkdownElementTypes.ATX_3 -> 2
                            MarkdownElementTypes.ATX_4 -> 3
                            MarkdownElementTypes.ATX_5 -> 4
                            else -> 5
                        }
                        result += TranscriptTextBlock(
                            annotated(node, MarkdownTokenTypes.ATX_CONTENT),
                            styles.headings[level],
                            TranscriptTextBlockKind.Heading,
                        )
                    }
                    MarkdownElementTypes.SETEXT_1,
                    MarkdownElementTypes.SETEXT_2 -> {
                        val level = if (node.type == MarkdownElementTypes.SETEXT_1) 0 else 1
                        result += TranscriptTextBlock(
                            annotated(node, MarkdownTokenTypes.SETEXT_CONTENT),
                            styles.headings[level],
                            TranscriptTextBlockKind.Heading,
                        )
                    }
                    MarkdownElementTypes.CODE_FENCE -> {
                        val language = node.findChildOfType(MarkdownTokenTypes.FENCE_LANG)
                        val minimumEnd = if (language != null && node.children.size > 3) 3 else 2
                        val content = if (node.children.size >= 3) {
                            val start = node.children[2].startOffset
                            val end = node.children[
                                (node.children.size - 2).coerceAtLeast(minimumEnd)
                            ].endOffset
                            text.substring(start, end).replaceIndent()
                        } else {
                            node.getTextInNode(text).toString().trim('`', '\n', '\r')
                        }
                        result += TranscriptTextBlock(
                            AnnotatedString(content),
                            styles.code,
                            TranscriptTextBlockKind.Code,
                        )
                    }
                    MarkdownElementTypes.CODE_BLOCK -> result += TranscriptTextBlock(
                        AnnotatedString(node.getTextInNode(text).toString().replaceIndent()),
                        styles.code,
                        TranscriptTextBlockKind.Code,
                    )
                    MarkdownElementTypes.BLOCK_QUOTE,
                    GFMElementTypes.ALERT -> result += TranscriptTextBlock(
                        flattened(node),
                        styles.body,
                        TranscriptTextBlockKind.Quote,
                    )
                    MarkdownElementTypes.ORDERED_LIST,
                    MarkdownElementTypes.UNORDERED_LIST -> result += TranscriptTextBlock(
                        flattened(node),
                        styles.body,
                        TranscriptTextBlockKind.Flow,
                    )
                    GFMElementTypes.TABLE -> result += TranscriptTextBlock(
                        flattened(node),
                        styles.code,
                        TranscriptTextBlockKind.Code,
                    )
                    MarkdownTokenTypes.HORIZONTAL_RULE -> result += TranscriptTextBlock(
                        AnnotatedString(""),
                        styles.body,
                        TranscriptTextBlockKind.Rule,
                    )
                    MarkdownElementTypes.IMAGE -> result += TranscriptTextBlock(
                        flattened(node),
                        styles.body,
                        TranscriptTextBlockKind.Flow,
                    )
                    MarkdownElementTypes.LINK_DEFINITION,
                    MarkdownTokenTypes.EOL -> Unit
                    else -> {
                        val content = flattened(node)
                        if (content.isNotEmpty()) {
                            result += TranscriptTextBlock(
                                content,
                                styles.body,
                                TranscriptTextBlockKind.Flow,
                            )
                        }
                    }
                }
            }
            if (result.isEmpty()) {
                result += TranscriptTextBlock(
                    text = AnnotatedString(text),
                    style = styles.body,
                    kind = TranscriptTextBlockKind.Flow,
                )
            }
            result
        }
    }

    return TranscriptTextDocument(blocks)
}

internal fun measureChatText(
    document: TranscriptTextDocument,
    maxWidth: Int,
    textMeasurer: androidx.compose.ui.text.TextMeasurer,
    styles: TranscriptTextStyles,
    density: Density,
): MeasuredTranscriptText {
    val codePadding = with(density) { styles.codePadding.roundToPx() }
    val quoteInset = with(density) {
        styles.quoteIndent.roundToPx() + styles.quoteBarWidth.roundToPx()
    }
    val measured = document.blocks.map { block ->
        if (block.kind == TranscriptTextBlockKind.Rule) {
            MeasuredTranscriptTextBlock(
                source = block,
                height = with(density) { 1.dp.roundToPx() },
            )
        } else {
            val flowWidth = when (block.kind) {
                TranscriptTextBlockKind.Quote -> (maxWidth - quoteInset).coerceAtLeast(0)
                else -> maxWidth
            }
            val softWrap = block.kind != TranscriptTextBlockKind.Code
            val layout = textMeasurer.measure(
                text = block.text,
                style = block.style,
                softWrap = softWrap,
                constraints = if (softWrap) {
                    Constraints(maxWidth = flowWidth)
                } else {
                    Constraints()
                },
                skipCache = true,
            )
            MeasuredTranscriptTextBlock(
                source = block,
                height = layout.size.height + if (block.kind == TranscriptTextBlockKind.Code) {
                    codePadding * 2
                } else {
                    0
                },
            )
        }
    }
    val spacing = with(density) { styles.blockSpacing.roundToPx() }
    return MeasuredTranscriptText(
        blocks = measured,
        height = measured.sumOf(MeasuredTranscriptTextBlock::height) +
            spacing * (measured.size - 1).coerceAtLeast(0),
    )
}

@Composable
internal fun ChatText(
    text: MeasuredTranscriptText,
    styles: TranscriptTextStyles,
    modifier: Modifier = Modifier,
) {
    val density = androidx.compose.ui.platform.LocalDensity.current
    Column(
        modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(styles.blockSpacing),
    ) {
        text.blocks.forEach { block ->
            val height = with(density) { block.height.toDp() }
            when (block.kind) {
                TranscriptTextBlockKind.Flow -> Text(
                    text = block.text,
                    style = block.style,
                    modifier = Modifier.fillMaxWidth().height(height),
                )
                TranscriptTextBlockKind.Heading -> Text(
                    text = block.text,
                    style = block.style,
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(height)
                        .semantics { heading() },
                )
                TranscriptTextBlockKind.Code -> Box(
                    Modifier
                        .fillMaxWidth()
                        .height(height)
                        .background(styles.codeBackground, MaterialTheme.shapes.small)
                        .horizontalScroll(rememberScrollState())
                        .padding(styles.codePadding),
                ) {
                    Text(
                        text = block.text,
                        style = block.style,
                        softWrap = false,
                    )
                }
                TranscriptTextBlockKind.Quote -> Row(
                    Modifier.fillMaxWidth().height(height),
                ) {
                    Box(
                        Modifier
                            .width(styles.quoteBarWidth)
                            .fillMaxHeight()
                            .background(styles.quoteBar),
                    )
                    Text(
                        text = block.text,
                        style = block.style,
                        modifier = Modifier
                            .padding(start = styles.quoteIndent)
                            .fillMaxWidth()
                            .height(height),
                    )
                }
                TranscriptTextBlockKind.Rule -> Box(
                    Modifier
                        .fillMaxWidth()
                        .height(height)
                        .background(MaterialTheme.colorScheme.outlineVariant),
                )
            }
        }
    }
}
