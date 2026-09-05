package app.tau

import androidx.sqlite.driver.bundled.BundledSQLiteDriver
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertSame
import kotlin.test.assertTrue

class TranscriptStoreTest {
    @Test
    fun retainsBranchesThinkingIdenticalPendingAndControlsAcrossRecreation() = runBlocking {
        val root = Files.createTempDirectory("tau-store-").toFile()
        val path = root.resolve("transcript.db").path
        var store = TranscriptStore({ path })
        try {
            val key = ChatKey(ConnectionSettings("http://one", "token-a").identity, "chat")
            val other = ChatKey(ConnectionSettings("http://one", "token-b").identity, "chat")
            assertTrue(store.chat(other).rows.isEmpty())
            val user = TranscriptEntry("user", role = EntryRole.User, content = listOf(EntryContent(ContentKind.Text, "original")))
            val hidden = TranscriptEntry("model", "user", entryType = "model_change")
            val alternate = TranscriptEntry("alternate", "model", role = EntryRole.User, content = listOf(EntryContent(ContentKind.Text, "other branch")))
            val live = TranscriptEntry("live-stream", "model", phase = EntryPhase.Live, origin = EntryOrigin(streamId = "stream"), role = EntryRole.Assistant,
                content = listOf(EntryContent(ContentKind.Thinking, "received")))
            val queue = QueueState(available = true, runId = "run", capabilities = listOf("queue_delete", "queue_run_prefix"), boundaries = listOf("reasoning_checkpoint", "turn"))
            store.applySnapshot(key, TranscriptCut("g", 40, "model", listOf(user, hidden, alternate, live), queue))
            val chat = store.chat(key)
            assertEquals(listOf("user", "live-stream"), chat.rows.map { it.entry.id })
            val liveRow = chat.rows.last()
            val first = store.beginSend(key, "same", "")
            val second = store.beginSend(key, "same", "")
            assertTrue(first.requestId != second.requestId)
            val queued = queue.copy(requests = listOf(QueuedRequest(first.requestId, 0, "steer", "prepared:first"), QueuedRequest(second.requestId, 1, "followUp", "prepared:second")))
            assertTrue(store.applyUpdates(key, listOf(
                TranscriptPatch("g", 41, TranscriptChange.Delta(live.id, 0, " thinking")),
                TranscriptPatch("g", 42, TranscriptChange.Queue(queued)),
            )))
            assertSame(liveRow, chat.rows.last())
            assertEquals("received thinking", liveRow.entry.content[0].text)
            assertEquals(listOf("prepared:first", "prepared:second"), chat.pending.map { it.text })
            val control = store.beginControl(key, "g") { state -> QueueOperation.Prefix(state.runId, state.requests.take(1).map { QueueRef(it.requestId, it.revision) }, "reasoning_checkpoint") }
            val waiting = queued.copy(control = QueueControl(control.commandId, "run", "prefix", "reasoning_checkpoint", listOf(QueueRef(first.requestId, 0)), "waiting"))
            store.applyUpdates(key, listOf(TranscriptPatch("g", 43, TranscriptChange.Queue(waiting))))
            store.setPreference(key, "draft", "retained draft")
            store.setPreference(key, "expanded:stream:stream", "true")
            store.select(key)
            assertFalse(store.applyUpdates(key, listOf(TranscriptPatch("g", 45, TranscriptChange.Delta(live.id, 0, "missed predecessor")))))
            assertEquals(43L, store.snapshot(key).sequence)
            assertEquals("received thinking", liveRow.entry.content[0].text)
            store.close()
            store = TranscriptStore({ path })
            store.disconnect(key.connection)
            val restored = store.chat(key)
            assertFalse(restored.synchronized)
            assertEquals("chat", store.loadConnection(key.connection).selected)
            assertEquals("received thinking", restored.rows.last().entry.content[0].text)
            assertEquals("retained draft", restored.preferences["draft"])
            assertEquals("true", restored.preferences["expanded:stream:stream"])
            assertEquals(listOf(SendStatus.Unconfirmed, SendStatus.Unconfirmed), restored.pending.map { it.status })
            assertEquals("unconfirmed", restored.controls.single().status)
            assertTrue(store.chat(other).rows.isEmpty())
            assertTrue(store.chat(other).pending.isEmpty())
            assertTrue(store.applySnapshot(key, TranscriptCut("g", 45, "model", listOf(user, hidden, alternate, live.copy(content = listOf(EntryContent(ContentKind.Thinking, "received thinking recovered")))), waiting)))
            assertEquals("waiting", restored.controls.single().status)
            assertEquals(1L, restored.pending.last().revision)
            assertFalse(store.applySnapshot(key, TranscriptCut("g", 40, "model", listOf(user, hidden), queue)))
            val saved = live.copy(id = "saved", phase = EntryPhase.Saved, content = listOf(EntryContent(ContentKind.Thinking, "received thinking recovered")))
            val delivered = TranscriptEntry("delivered", "saved", origin = EntryOrigin(requestId = first.requestId, requestRevision = 0), role = EntryRole.User,
                content = listOf(EntryContent(ContentKind.Text, "prepared:first")))
            val savedRow = restored.rows.last()
            assertTrue(store.applyUpdates(key, listOf(
                TranscriptPatch("g", 46, TranscriptChange.Entry(saved)),
                TranscriptPatch("g", 47, TranscriptChange.Entry(delivered)),
                TranscriptPatch("g", 48, TranscriptChange.Queue(waiting.copy(paused = true, requests = waiting.requests.drop(1), control = waiting.control!!.copy(status = "applied")))),
            )))
            assertSame(savedRow, restored.rows.first { it.entry.id == "saved" })
            assertEquals(listOf(second.requestId), restored.pending.map { it.requestId })
            store.acknowledge(key.connection, first.requestId, true, disposition = "submitted")
            assertEquals(listOf(second.requestId), restored.pending.map { it.requestId })
            val tail = live.copy(id = "live-tail", parentId = "delivered", origin = EntryOrigin(streamId = "tail"))
            store.applyUpdates(key, listOf(TranscriptPatch("g", 49, TranscriptChange.Entry(tail))))
            store.applySnapshot(key, TranscriptCut("replacement", 0, "delivered", listOf(user, hidden, alternate, saved, delivered), QueueState()))
            assertEquals(EntryPhase.Interrupted, restored.rows.last().entry.phase)
            assertEquals("received", restored.rows.last().entry.content[0].text)
            assertEquals(SendStatus.Unconfirmed, restored.pending.single().status)
            store.applyUpdates(key, listOf(TranscriptPatch("replacement", 1, TranscriptChange.Head("alternate"))))
            assertEquals(listOf("user", "alternate"), restored.rows.map { it.entry.id })
            assertTrue(store.snapshot(key).entries.any { it.id == "live-tail" })
        } finally { store.close(); root.deleteRecursively() }
    }

    @Test
    fun commitsCursorAndContentTogetherAndRetainsFilesAndUncertainOperations() = runBlocking {
        val root = Files.createTempDirectory("tau-store-atomic-").toFile()
        val path = root.resolve("transcript.db").path
        var store = TranscriptStore({ path })
        val key = ChatKey("connection", "chat")
        try {
            val live = TranscriptEntry("live-s", phase = EntryPhase.Live, origin = EntryOrigin(streamId = "s"), role = EntryRole.Assistant,
                content = listOf(EntryContent(ContentKind.Thinking, "before")))
            store.applySnapshot(key, TranscriptCut("g", 1, entries = listOf(live), queue = QueueState(available = true)))
            BundledSQLiteDriver().open(path).use { db ->
                db.prepare("CREATE TRIGGER reject_position BEFORE UPDATE ON records WHEN NEW.kind='position' BEGIN SELECT RAISE(ABORT,'injected write failure'); END").use { it.step() }
            }
            assertFailsWith<Exception> { store.applyUpdates(key, listOf(TranscriptPatch("g", 2, TranscriptChange.Delta(live.id, 0, " after")))) }
            assertEquals(1L, store.snapshot(key).sequence)
            assertEquals("before", store.chat(key).rows.single().entry.content[0].text)
            store.close()
            store = TranscriptStore({ path })
            assertEquals(1L, store.snapshot(key).sequence)
            assertEquals("before", store.chat(key).rows.single().entry.content[0].text)
            BundledSQLiteDriver().open(path).use { db -> db.prepare("DROP TRIGGER reject_position").use { it.step() } }
            store.applySnapshot(key, TranscriptCut("g", 1, entries = listOf(live), queue = QueueState(available = true)))
            store.addFiles(key, listOf(PickedFile("retained.txt", "file content".encodeToByteArray())))
            val send = store.beginSend(key, "file prompt", "next draft")
            val control = store.beginControl(key, "g") { QueueOperation.Pause(null, "turn") }
            store.close()
            store = TranscriptStore({ path })
            store.disconnect(key.connection)
            val chat = store.chat(key)
            assertEquals(SendStatus.Rejected, chat.pending.single().status)
            assertEquals("unconfirmed", chat.controls.single().status)
            assertEquals(control.commandId, chat.controls.single().commandId)
            assertEquals("file content", store.readFile(key, send.files.single()).bytes.decodeToString())
            store.restoreSend(key, send.requestId)
            assertTrue(chat.pending.isEmpty())
            assertEquals("file prompt\n\nnext draft", chat.preferences["draft"])
            assertEquals(send.files.single(), chat.files.single())
            val again = store.beginSend(key, "file prompt", "")
            store.updateSend(key, again.copy(wireText = "prepared file prompt", status = SendStatus.Sending))
            store.acknowledge(key.connection, again.requestId, false, uncertain = true, detail = "Pipe closed")
            assertEquals(SendStatus.Unconfirmed, chat.pending.single().status)
            assertFailsWith<IllegalArgumentException> { store.restoreSend(key, again.requestId) }
            assertEquals("file content", store.readFile(key, again.files.single()).bytes.decodeToString())
            store.acknowledge(key.connection, again.requestId, false, detail = "Rejected")
            assertEquals(SendStatus.Rejected, chat.pending.single().status)
            store.removeChat(key)
            assertTrue(store.chat(key).rows.isEmpty())
            assertTrue(store.chat(key).pending.isEmpty())
        } finally { store.close(); root.deleteRecursively() }
    }

    @Test
    fun streamsIntoOneStableRowWithoutRewritingTheLongHistory() = runBlocking {
        val store = TranscriptStore({ ":memory:" })
        try {
            val key = ChatKey("long", "chat")
            val entries = (0 until 10_000).map { index -> TranscriptEntry("e$index", if (index == 0) null else "e${index - 1}", role = EntryRole.User,
                content = listOf(EntryContent(ContentKind.Text, "message $index"))) }
            val live = TranscriptEntry("live-tail", entries.last().id, phase = EntryPhase.Live, origin = EntryOrigin(streamId = "tail"), role = EntryRole.Assistant,
                content = listOf(EntryContent(ContentKind.Thinking, "")))
            store.applySnapshot(key, TranscriptCut("g", 0, entries.last().id, entries + live, QueueState()))
            val chat = store.chat(key)
            val firstRow = chat.rows.first()
            val firstEntry = firstRow.entry
            val tail = chat.rows.last()
            val patches = (1L..1000L).map { TranscriptPatch("g", it, TranscriptChange.Delta(live.id, 0, "thinking \uD83E\uDDE0 ")) }
            for (batch in patches.chunked(32)) assertTrue(store.applyUpdates(key, batch))
            assertEquals(10_001, chat.rows.size)
            assertSame(firstRow, chat.rows.first())
            assertSame(firstEntry, firstRow.entry)
            assertSame(tail, chat.rows.last())
            assertEquals("thinking \uD83E\uDDE0 ".repeat(1000), tail.entry.content[0].text)
            assertEquals(1000L, store.snapshot(key).sequence)
            assertEquals(10_001, store.snapshot(key).entries.size)
            assertTrue(store.applyUpdates(key, listOf(patches.last())))
            assertEquals("thinking \uD83E\uDDE0 ".repeat(1000), tail.entry.content[0].text)
        } finally { store.close() }
    }
}
