package app.tau

internal data class TranscriptGroup(val key: String, val rows: List<EventRow>, val parts: List<TranscriptPart>)

internal sealed interface TranscriptPart {
    data class Text(val row: EventRow) : TranscriptPart
    data class Details(val key: String, val blocks: List<TranscriptDetail>) : TranscriptPart
    data class Attachment(val row: EventRow) : TranscriptPart
    data class Failure(val row: EventRow) : TranscriptPart
}

internal data class TranscriptDetail(val row: EventRow, val results: List<EventRow> = emptyList(), val key: String = "tool:${row.key}")

internal fun presentTranscript(rows: List<EventRow>, previous: List<TranscriptGroup> = emptyList()): List<TranscriptGroup> {
    val priorGroups = previous.flatMap { group -> group.rows.map { it.key to group } }.toMap()
    val groups = mutableListOf<List<EventRow>>()
    val results = mutableMapOf<String, MutableList<EventRow>>()
    val calls = mutableSetOf<String>()
    var group = mutableListOf<EventRow>()
    for (row in rows) {
        val event = row.event
        if (event.role == EventRole.Tool) event.toolCallId?.let { results.getOrPut(it) { mutableListOf() }.add(row) }
        if (event.kind == EventKind.Tool) event.toolCallId?.let(calls::add)
        val prior = group.lastOrNull()?.event
        val response = event.role == EventRole.Assistant || event.role == EventRole.Tool
        val priorResponse = prior?.role == EventRole.Assistant || prior?.role == EventRole.Tool
        if (group.isNotEmpty() && !(response && priorResponse || event.role == prior?.role && event.entryId == prior.entryId)) {
            groups.add(group); group = mutableListOf()
        }
        group.add(row)
    }
    if (group.isNotEmpty()) groups.add(group)
    return groups.map { members ->
        val prior = members.firstNotNullOfOrNull { priorGroups[it.key] }
        val priorBlocks = mutableMapOf<String, String>()
        val priorDetails = mutableMapOf<String, String>()
        for (part in prior?.parts.orEmpty()) if (part is TranscriptPart.Details) {
            priorDetails[part.blocks.first().key] = part.key
            for (block in part.blocks) for (row in listOf(block.row) + block.results) priorBlocks[row.key] = block.key
        }
        val parts = buildList {
            var details = mutableListOf<TranscriptDetail>()
            fun flushDetails() {
                if (details.isEmpty()) return
                val blocks = details.map { block ->
                    val key = (listOf(block.row) + block.results).firstNotNullOfOrNull { priorBlocks[it.key] }
                    if (key == null) block else block.copy(key = key)
                }
                val key = blocks.firstNotNullOfOrNull { priorDetails[it.key] } ?: "details:${blocks.first().row.key}"
                add(TranscriptPart.Details(key, blocks)); details = mutableListOf()
            }
            for (row in members) {
                val event = row.event
                if (event.role == EventRole.Tool) {
                    val output = results[event.toolCallId].orEmpty().ifEmpty { listOf(row) }
                    if (event.toolCallId !in calls && output.first() === row) details.add(TranscriptDetail(row, output))
                } else when (event.kind) {
                    EventKind.Text, EventKind.Image -> if (!(event.kind == EventKind.Image && event.attachment != null || event.kind == EventKind.Text && event.text.isEmpty())) {
                        flushDetails(); add(TranscriptPart.Text(row))
                    }
                    EventKind.Thinking -> if (event.text.isNotEmpty()) details.add(TranscriptDetail(row))
                    EventKind.Tool -> details.add(TranscriptDetail(row, results[event.toolCallId].orEmpty()))
                    EventKind.Hidden -> Unit
                }
                if ((event.phase == EventPhase.Interrupted || event.stopReason == "aborted" || event.stopReason == "error" || event.isError && event.role != EventRole.Tool) &&
                    members.lastOrNull { it.event.entryId == event.entryId } === row) {
                    flushDetails(); add(TranscriptPart.Failure(row))
                }
                if (event.attachment != null) { flushDetails(); add(TranscriptPart.Attachment(row)) }
            }
            flushDetails()
        }
        TranscriptGroup(prior?.key ?: members.first().key, members, parts)
    }
}

internal fun RetainedChat?.latestResponseFailed(): Boolean {
    val latest = this?.rows?.asReversed()?.firstOrNull {
        it.event.role == EventRole.User || it.event.role == EventRole.Assistant
    }?.event ?: return false
    return latest.role == EventRole.Assistant && latest.phase != EventPhase.Live && latest.stopReason == "error"
}
