package app.tau

import kotlin.test.Test
import kotlin.test.assertEquals

class TauControllerTest {
    @Test
    fun keeps_live_attempt_in_chronological_place_when_steering_arrives() {
        val live = ChatAttempt(
            entryId = "live-2",
            timestampMs = 2,
            content = listOf(
                ChatContent(
                    kind = ChatContentKind.Text,
                    contentIndex = 0,
                    text = "Work already streamed",
                    hasContent = true,
                ),
            ),
        )
        val messages = listOf(
            ChatMessage("u1", ChatRole.User, "Start", timestampMs = 1),
            ChatMessage("u2", ChatRole.User, "Steer", timestampMs = 3),
        )

        val merged = mergeLiveAttempts(messages, listOf(live))

        assertEquals(
            listOf(ChatRole.User, ChatRole.Assistant, ChatRole.User),
            merged.map(ChatMessage::role),
        )
        assertEquals("Work already streamed", merged[1].attempts.single().content.single().text)
    }

    @Test
    fun canonical_reconciliation_keeps_live_detail_content_in_the_same_attempt() {
        val live = ChatAttempt(
            entryId = "live-2",
            timestampMs = 2,
            content = listOf(
                ChatContent(
                    kind = ChatContentKind.Thinking,
                    contentIndex = 0,
                    text = "Visible planning",
                    hasContent = true,
                ),
            ),
        )
        val canonical = ChatMessage(
            entryId = "a1",
            role = ChatRole.Assistant,
            text = "",
            timestampMs = 2,
            attempts = listOf(
                ChatAttempt(
                    entryId = "a1",
                    timestampMs = 2,
                    stopReason = "error",
                    errorMessage = "terminated",
                    content = listOf(
                        ChatContent(
                            kind = ChatContentKind.Thinking,
                            contentIndex = 0,
                            detailIndex = 0,
                            hasContent = true,
                        ),
                    ),
                ),
            ),
        )

        val first = preserveAttemptContent(listOf(canonical), emptyList(), listOf(live))
        val second = preserveAttemptContent(listOf(canonical), first, emptyList())

        assertEquals("Visible planning", first.single().attempts.single().content.single().text)
        assertEquals("Visible planning", second.single().attempts.single().content.single().text)
        assertEquals("terminated", second.single().attempts.single().errorMessage)
    }

    @Test
    fun reconciles_acknowledged_messages_in_order_after_their_baseline() {
        val first = OutgoingMessage(
            requestId = "first",
            text = "Original prompt",
            canonicalText = "Original prompt",
            afterEntryId = "baseline",
            occurrence = 0,
            canonicalOccurrence = 0,
        )
        val second = first.copy(
            requestId = "second",
            text = "Different prompt",
            canonicalText = "Different canonical text",
            occurrence = 1,
        )
        val before = listOf(
            ChatMessage("old", ChatRole.User, "Earlier prompt"),
            ChatMessage("baseline", ChatRole.Assistant, "Ready"),
        )
        assertEquals(listOf(first, second), reconcileOutgoingMessages(listOf(first, second), before))

        val afterFirst = before + ChatMessage("accepted-1", ChatRole.User, "Expanded prompt text")
        assertEquals(listOf(second), reconcileOutgoingMessages(listOf(first, second), afterFirst))
        assertEquals(listOf(second), reconcileOutgoingMessages(listOf(second), afterFirst))

        val afterBoth = afterFirst + ChatMessage("accepted-2", ChatRole.User, "Different canonical text")
        assertEquals(emptyList(), reconcileOutgoingMessages(listOf(second), afterBoth))
    }

    @Test
    fun reconciles_exact_canonical_prompt_when_a_transient_baseline_disappears() {
        val outgoing = OutgoingMessage(
            requestId = "accepted",
            text = "Prompt with attachment",
            canonicalText = "Prompt with attachment\n\nAttached files are available at:\n- file: /upload/file",
            afterEntryId = "replaced-details-row",
            occurrence = 0,
            canonicalOccurrence = 0,
        )
        val history = listOf(
            ChatMessage("older", ChatRole.Assistant, "Earlier answer"),
            ChatMessage("accepted", ChatRole.User, outgoing.canonicalText),
        )

        assertEquals(emptyList(), reconcileOutgoingMessages(listOf(outgoing), history))
    }

    @Test
    fun canonical_prompt_occurrence_does_not_match_an_older_duplicate() {
        val outgoing = OutgoingMessage(
            requestId = "duplicate",
            text = "Run it again",
            canonicalText = "Run it again",
            afterEntryId = "replaced-details-row",
            occurrence = 0,
            canonicalOccurrence = 1,
        )
        val before = listOf(ChatMessage("old", ChatRole.User, "Run it again"))
        val after = before + ChatMessage("new", ChatRole.User, "Run it again")

        assertEquals(listOf(outgoing), reconcileOutgoingMessages(listOf(outgoing), before))
        assertEquals(emptyList(), reconcileOutgoingMessages(listOf(outgoing), after))
    }
}
