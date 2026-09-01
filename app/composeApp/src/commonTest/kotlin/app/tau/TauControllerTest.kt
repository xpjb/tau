package app.tau

import kotlin.test.Test
import kotlin.test.assertEquals

class TauControllerTest {
    @Test
    fun reconciles_acknowledged_messages_once_after_their_baseline() {
        val first = OutgoingMessage(
            requestId = "first",
            text = "Repeat",
            sentText = "Repeat",
            afterEntryId = "baseline",
            occurrence = 0,
        )
        val second = first.copy(requestId = "second", occurrence = 1)
        val before = listOf(
            ChatMessage("old", ChatRole.User, "Repeat"),
            ChatMessage("baseline", ChatRole.Assistant, "Ready"),
        )
        assertEquals(listOf(first, second), reconcileOutgoingMessages(listOf(first, second), before))

        val afterFirst = before + ChatMessage("accepted-1", ChatRole.User, "Repeat")
        assertEquals(listOf(second), reconcileOutgoingMessages(listOf(first, second), afterFirst))
        assertEquals(listOf(second), reconcileOutgoingMessages(listOf(second), afterFirst))

        val afterBoth = afterFirst + ChatMessage("accepted-2", ChatRole.User, "Repeat")
        assertEquals(emptyList(), reconcileOutgoingMessages(listOf(second), afterBoth))

        val missingBaseline = first.copy(requestId = "missing", afterEntryId = "compacted")
        assertEquals(
            listOf(missingBaseline),
            reconcileOutgoingMessages(listOf(missingBaseline), afterBoth),
        )
    }
}
