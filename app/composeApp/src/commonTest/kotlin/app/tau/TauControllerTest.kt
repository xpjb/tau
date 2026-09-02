package app.tau

import kotlin.test.Test
import kotlin.test.assertEquals

class TauControllerTest {
    @Test
    fun reconciles_acknowledged_messages_in_order_after_their_baseline() {
        val first = OutgoingMessage(
            requestId = "first",
            text = "Original prompt",
            afterEntryId = "baseline",
            occurrence = 0,
        )
        val second = first.copy(requestId = "second", text = "Different prompt", occurrence = 1)
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

        val missingBaseline = first.copy(requestId = "missing", afterEntryId = "compacted")
        assertEquals(
            listOf(missingBaseline),
            reconcileOutgoingMessages(listOf(missingBaseline), afterBoth),
        )
    }
}
