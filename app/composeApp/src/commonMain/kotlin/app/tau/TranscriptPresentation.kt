package app.tau

internal data class TranscriptGroup(val rows: List<EntryRow>) {
    val key: String get() = rows.first().key
}

internal data class TranscriptPresentation(
    val groups: List<TranscriptGroup>,
    val results: Map<String, EntryRow>,
    val calls: Set<String>,
)

internal sealed interface TranscriptPart {
    data class Text(val row: EntryRow, val index: Int) : TranscriptPart
    data class Details(val blocks: List<TranscriptDetail>) : TranscriptPart {
        val key: String get() = "details:${blocks.first().row.key}:${blocks.first().index}"
    }
    data class Attachment(val row: EntryRow) : TranscriptPart
    data class Failure(val row: EntryRow) : TranscriptPart
}

internal data class TranscriptDetail(val row: EntryRow, val index: Int, val result: EntryRow? = null) {
    val key: String get() = "tool:${row.key}:$index"
}

internal fun presentTranscript(rows: List<EntryRow>): TranscriptPresentation {
    val groups = mutableListOf<TranscriptGroup>()
    val results = mutableMapOf<String, EntryRow>()
    val calls = mutableSetOf<String>()
    var response = mutableListOf<EntryRow>()
    for (row in rows) {
        val entry = row.entry
        if (entry.role == EntryRole.Tool) entry.toolCallId?.let { results[it] = row }
        for (content in entry.content) if (content.kind == ContentKind.Tool) content.toolCallId?.let(calls::add)
        if (entry.role == EntryRole.Assistant || entry.role == EntryRole.Tool) response.add(row)
        else {
            if (response.isNotEmpty()) { groups.add(TranscriptGroup(response)); response = mutableListOf() }
            groups.add(TranscriptGroup(listOf(row)))
        }
    }
    if (response.isNotEmpty()) groups.add(TranscriptGroup(response))
    return TranscriptPresentation(groups, results, calls)
}

internal fun transcriptParts(group: TranscriptGroup, presentation: TranscriptPresentation): List<TranscriptPart> = buildList {
    var details = mutableListOf<TranscriptDetail>()
    fun flushDetails() {
        if (details.isNotEmpty()) { add(TranscriptPart.Details(details)); details = mutableListOf() }
    }
    for (row in group.rows) {
        val entry = row.entry
        if (entry.role == EntryRole.Tool) {
            if (entry.toolCallId !in presentation.calls) details.add(TranscriptDetail(row, -1, row))
        } else for ((index, content) in entry.content.withIndex()) {
            when (content.kind) {
                ContentKind.Text, ContentKind.Image -> {
                    if (content.kind == ContentKind.Image && entry.attachment != null || content.kind == ContentKind.Text && content.text.isEmpty()) continue
                    flushDetails()
                    add(TranscriptPart.Text(row, index))
                }
                ContentKind.Thinking -> if (content.text.isNotEmpty()) details.add(TranscriptDetail(row, index))
                ContentKind.Tool -> details.add(TranscriptDetail(row, index, presentation.results[content.toolCallId]))
                ContentKind.Hidden -> Unit
            }
        }
        if (entry.phase == EntryPhase.Interrupted || entry.stopReason == "aborted" || entry.stopReason == "error" || entry.isError && entry.role != EntryRole.Tool) {
            flushDetails()
            add(TranscriptPart.Failure(row))
        }
        if (entry.attachment != null) {
            flushDetails()
            add(TranscriptPart.Attachment(row))
        }
    }
    flushDetails()
}
