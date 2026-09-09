package app.tau

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertSame

class TranscriptPresentationTest {
    @Test
    fun groups_flat_events_and_pairs_tools_across_history_batches() {
        val events = listOf(
            TranscriptEvent("u:0", 0, "u", role = EventRole.User, kind = EventKind.Text, text = "Check"),
            TranscriptEvent("s:0", 1, "live-s", phase = EventPhase.Live, role = EventRole.Assistant, kind = EventKind.Text, text = "Before"),
            TranscriptEvent("s:1", 2, "live-s", phase = EventPhase.Live, role = EventRole.Assistant, kind = EventKind.Thinking, text = "Thinking"),
            TranscriptEvent("s:2", 3, "live-s", phase = EventPhase.Live, role = EventRole.Assistant, kind = EventKind.Tool, text = "input one", toolCallId = "one", toolName = "bash"),
            TranscriptEvent("s:3", 4, "live-s", phase = EventPhase.Live, role = EventRole.Assistant, kind = EventKind.Tool, text = "input two", toolCallId = "two", toolName = "bash"),
            TranscriptEvent("t2:0", 5, "t2", role = EventRole.Tool, kind = EventKind.Text, text = "output two", toolCallId = "two"),
            TranscriptEvent("t1:0", 6, "t1", role = EventRole.Tool, kind = EventKind.Text, text = "output one", toolCallId = "one"),
            TranscriptEvent("t1:1", 7, "t1", role = EventRole.Tool, kind = EventKind.Text, text = "more output", toolCallId = "one"),
            TranscriptEvent("a:0", 8, "a", role = EventRole.Assistant, kind = EventKind.Text, text = "After"),
            TranscriptEvent("a:1", 9, "a", role = EventRole.Assistant, kind = EventKind.Thinking, text = "Later details"),
        )
        val rows = events.map(::EventRow)
        val recent = presentTranscript(rows.drop(5))
        assertEquals(2, assertIs<TranscriptPart.Details>(recent.single().parts.first()).blocks.size)
        val presentation = presentTranscript(rows, recent)
        assertEquals(2, presentation.size)
        assertEquals(recent.single().key, presentation.last().key)
        val parts = presentation.last().parts
        assertEquals(listOf("Text", "Details", "Text", "Details"), parts.map { it::class.simpleName })
        assertSame(rows[1], assertIs<TranscriptPart.Text>(parts.first()).row)
        val details = assertIs<TranscriptPart.Details>(parts[1])
        val recentDetails = assertIs<TranscriptPart.Details>(recent.single().parts.first())
        assertEquals(recentDetails.key, details.key)
        assertEquals(recentDetails.blocks.first().key, details.blocks.last().key)
        assertEquals(listOf(rows[2], rows[3], rows[4]), details.blocks.map { it.row })
        assertEquals(listOf(rows[6], rows[7]), details.blocks[1].results)
        assertEquals(listOf(rows[5]), details.blocks[2].results)
        assertEquals(events, rows.map { it.event })
        val groupKey = presentation.last().key
        val detailKey = details.key
        for (row in rows.subList(1, 5)) row.event = row.event.copy(entryId = "saved-s", phase = EventPhase.Saved)
        val saved = presentTranscript(rows, presentation)
        assertEquals(groupKey, saved.last().key)
        assertEquals(detailKey, assertIs<TranscriptPart.Details>(saved.last().parts[1]).key)
        assertSame(rows[1], saved.last().rows.first())
        val attachment = EventRow(TranscriptEvent("file:0", 10, "file", role = EventRole.Tool, kind = EventKind.Text,
            text = "Output", toolCallId = "missing", isError = true, attachment = ChatAttachment(AttachmentKind.File, "result.txt")))
        val appended = presentTranscript(rows + attachment, saved)
        assertSame(attachment, assertIs<TranscriptPart.Attachment>(appended.last().parts.last()).row)
    }
}
