package app.tau

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertSame

class TranscriptPresentationTest {
    @Test
    fun keeps_text_details_and_paired_tools_in_order_without_changing_entries() {
        val user = EntryRow(TranscriptEntry("u", role = EntryRole.User, content = listOf(EntryContent(ContentKind.Text, "Check"))))
        val assistant = EntryRow(TranscriptEntry("live-a", "u", phase = EntryPhase.Live, origin = EntryOrigin(streamId = "a"), role = EntryRole.Assistant,
            content = listOf(EntryContent(ContentKind.Text, "Before"), EntryContent(ContentKind.Thinking, "Thinking"),
                EntryContent(ContentKind.Tool, "input one", "one", "bash"), EntryContent(ContentKind.Tool, "input two", "two", "bash"))))
        val second = EntryRow(TranscriptEntry("t2", role = EntryRole.Tool, toolCallId = "two", content = listOf(EntryContent(ContentKind.Text, "output two"))))
        val first = EntryRow(TranscriptEntry("t1", role = EntryRole.Tool, toolCallId = "one", content = listOf(EntryContent(ContentKind.Text, "output one"))))
        val answer = EntryRow(TranscriptEntry("a2", role = EntryRole.Assistant, content = listOf(EntryContent(ContentKind.Text, "After"), EntryContent(ContentKind.Thinking, "Later details"))))
        val rows = listOf(user, assistant, second, first, answer)
        val entries = rows.map { it.entry }
        val presentation = presentTranscript(rows)
        assertEquals(2, presentation.groups.size)
        val group = presentation.groups.last()
        val parts = transcriptParts(group, presentation)
        assertEquals(listOf("Text", "Details", "Text", "Details"), parts.map { it::class.simpleName })
        assertEquals(0, assertIs<TranscriptPart.Text>(parts.first()).index)
        val details = assertIs<TranscriptPart.Details>(parts[1])
        assertEquals(listOf(1, 2, 3), details.blocks.map { it.index })
        assertSame(first, details.blocks[1].result)
        assertSame(second, details.blocks[2].result)
        assertEquals(entries, rows.map { it.entry })
        val groupKey = group.key
        val detailKey = details.key
        assistant.entry = assistant.entry.copy(id = "a1", phase = EntryPhase.Saved)
        val saved = presentTranscript(rows)
        assertEquals(groupKey, saved.groups.last().key)
        assertEquals(detailKey, assertIs<TranscriptPart.Details>(transcriptParts(saved.groups.last(), saved)[1]).key)
        assertSame(assistant, saved.groups.last().rows.first())

        val notice = EntryRow(TranscriptEntry("n", role = EntryRole.System, content = listOf(EntryContent(ContentKind.Text, "Notice"))))
        val interrupted = EntryRow(TranscriptEntry("lost", role = EntryRole.Assistant, phase = EntryPhase.Interrupted, content = listOf(EntryContent(ContentKind.Thinking, "Partial"))))
        val attachment = EntryRow(TranscriptEntry("file", role = EntryRole.Tool, toolCallId = "missing", isError = true,
            content = listOf(EntryContent(ContentKind.Text, "Orphan output")), attachment = ChatAttachment(AttachmentKind.File, "result.txt")))
        val mixed = presentTranscript(listOf(user, assistant, notice, first, interrupted, attachment))
        assertSame(first, assertIs<TranscriptPart.Details>(transcriptParts(mixed.groups[1], mixed)[1]).blocks[1].result)
        val tail = transcriptParts(mixed.groups.last(), mixed)
        assertEquals(listOf("Details", "Failure", "Details", "Attachment"), tail.map { it::class.simpleName })
        assertSame(attachment, assertIs<TranscriptPart.Details>(tail[2]).blocks.single().result)
        assertSame(attachment, assertIs<TranscriptPart.Attachment>(tail.last()).row)
    }
}
