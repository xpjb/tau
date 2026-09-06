package app.tau

import androidx.sqlite.driver.bundled.BundledSQLiteDriver
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.encodeToString
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertSame
import kotlin.test.assertTrue

class TranscriptStoreTest {
    @Test
    fun migratesSessionMetadataAndWritesOnlyChangedRows() = runBlocking {
        val root = Files.createTempDirectory("tau-session-rows-").toFile()
        val path = root.resolve("transcript.db").path
        val key = ChatKey("account-a", "chat")
        val first = SessionSummary(key.session, "Chat 🧠", SessionStatus.Idle, "Ready", SessionModel("provider", "model"), "parent", 1, 2, ContextUsage(64000, 200000))
        val sessions = listOf(first) + (1..200).map { first.copy(id = "other-$it", model = null, parentId = null, contextUsage = null) }
        val other = first.copy(title = "Other account", contextUsage = ContextUsage(null, 128000))
        val entry = TranscriptEntry("saved", role = EntryRole.User, content = listOf(EntryContent(ContentKind.Text, "Retained history")))
        var store = TranscriptStore({ path })
        try {
            BundledSQLiteDriver().open(path).use { db ->
                for (sql in listOf(
                    "CREATE TABLE records (connection TEXT NOT NULL, chat TEXT NOT NULL, kind TEXT NOT NULL, id TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(connection,chat,kind,id))",
                    "CREATE TABLE files (connection TEXT NOT NULL, chat TEXT NOT NULL, id TEXT NOT NULL, owner TEXT NOT NULL, name TEXT NOT NULL, size INTEGER NOT NULL, body BLOB NOT NULL, PRIMARY KEY(connection,chat,id))",
                    "PRAGMA user_version=1",
                )) db.prepare(sql).use { it.step() }
                db.prepare("INSERT INTO records VALUES (?,?,?,?,?)").use { statement ->
                    for (row in listOf(
                        listOf(key.connection, "", "connection", "sessions", TauJson.encodeToString(sessions)),
                        listOf("account-b", "", "connection", "sessions", TauJson.encodeToString(listOf(other))),
                        listOf(key.connection, "", "connection", "selected", key.session),
                        listOf(key.connection, key.session, "entry", entry.id, TauJson.encodeToString(entry)),
                        listOf(key.connection, key.session, "position", "current", TauJson.encodeToString(StoredPosition("g", 4, entry.id))),
                        listOf(key.connection, key.session, "preference", "draft", "Retained draft"),
                    )) {
                        row.forEachIndexed { index, value -> statement.bindText(index + 1, value) }
                        statement.step(); statement.reset()
                    }
                }
                db.prepare("INSERT INTO files VALUES ('account-a','chat','file','','kept.txt',4,?)").use { it.bindBlob(1, byteArrayOf(1, 2, 3, 4)); it.step() }
                db.prepare("CREATE TRIGGER reject_import BEFORE DELETE ON records WHEN OLD.kind='connection' AND OLD.id='sessions' BEGIN SELECT RAISE(ABORT,'injected import failure'); END").use { it.step() }
                assertFailsWith<Exception> { store.loadConnection(key.connection) }
                db.prepare("PRAGMA user_version").use { it.step(); assertEquals(1, it.getInt(0)) }
                db.prepare("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='sessions'").use { it.step(); assertEquals(0, it.getInt(0)) }
                db.prepare("DROP TRIGGER reject_import").use { it.step() }
                assertEquals(StoredConnection(sessions, key.session), store.loadConnection(key.connection))
                assertEquals(listOf(other), store.loadConnection("account-b").sessions)
                db.prepare("PRAGMA user_version").use { it.step(); assertEquals(3, it.getInt(0)) }
                db.prepare("SELECT count(*) FROM records WHERE kind='connection' AND id='sessions'").use { it.step(); assertEquals(0, it.getInt(0)) }
                for (table in listOf("records", "files")) for (operation in listOf("INSERT", "UPDATE", "DELETE")) {
                    db.prepare("CREATE TRIGGER protect_${table}_$operation BEFORE $operation ON $table BEGIN SELECT RAISE(ABORT,'unrelated data write'); END").use { it.step() }
                }
                db.prepare("CREATE TABLE writes (operation TEXT,connection TEXT,id TEXT)").use { it.step() }
                for (operation in listOf("INSERT", "UPDATE", "DELETE")) {
                    val row = if (operation == "DELETE") "OLD" else "NEW"
                    db.prepare("CREATE TRIGGER count_$operation AFTER $operation ON sessions BEGIN INSERT INTO writes VALUES ('$operation',$row.connection,$row.id); END").use { it.step() }
                }
                store.saveSessions(key.connection, sessions)
                db.prepare("SELECT count(*) FROM writes").use { it.step(); assertEquals(0, it.getInt(0)) }
                val state = SessionState(key.session, SessionStatus.Running, "Generating", ContextUsage(null, 128000))
                store.updateSessionState(key.connection, state)
                store.updateSessionState(key.connection, state)
                store.updateSessionState(key.connection, state.copy(sessionId = "missing"))
                var expected = listOf(first.copy(status = state.status, detail = state.detail, contextUsage = state.contextUsage)) + sessions.drop(1)
                store.saveSessions(key.connection, expected)
                db.prepare("SELECT operation,connection,id FROM writes").use {
                    assertTrue(it.step()); assertEquals("UPDATE", it.getText(0)); assertEquals(key.connection, it.getText(1)); assertEquals(key.session, it.getText(2)); assertFalse(it.step())
                }
                assertEquals(expected, store.loadConnection(key.connection).sessions)
                assertEquals(listOf(other), store.loadConnection("account-b").sessions)
                db.prepare("DELETE FROM writes").use { it.step() }
                db.prepare("CREATE TRIGGER reject_title BEFORE UPDATE ON sessions WHEN NEW.title='Rejected title' BEGIN SELECT RAISE(ABORT,'injected snapshot failure'); END").use { it.step() }
                assertFailsWith<Exception> { store.saveSessions(key.connection, listOf(expected.first().copy(title = "Rejected title")) + expected.drop(1).dropLast(1)) }
                assertEquals(expected, store.loadConnection(key.connection).sessions)
                db.prepare("SELECT count(*) FROM writes").use { it.step(); assertEquals(0, it.getInt(0)) }
                db.prepare("DROP TRIGGER reject_title").use { it.step() }
                expected = listOf(expected.first().copy(title = "Renamed", model = null, parentId = null, contextUsage = null)) + expected.drop(1).dropLast(1) + other.copy(id = "new")
                store.saveSessions(key.connection, expected)
                db.prepare("SELECT count(*) FROM writes").use { it.step(); assertEquals(3, it.getInt(0)) }
                store.saveSessions(key.connection, expected)
                db.prepare("SELECT count(*) FROM writes").use { it.step(); assertEquals(3, it.getInt(0)) }
                expected = expected.reversed()
                store.saveSessions(key.connection, expected)
                store.close()
                store = TranscriptStore({ path })
                assertEquals(StoredConnection(expected, key.session), store.loadConnection(key.connection))
                assertTrue(store.snapshot(key).entries.isEmpty())
                assertEquals(0L, store.snapshot(key).sequence)
                assertEquals("Retained draft", store.chat(key).preferences["draft"])
                assertTrue(store.readFile(key, store.chat(key).files.single()).bytes.contentEquals(byteArrayOf(1, 2, 3, 4)))
                store.saveSessions(key.connection, emptyList())
                assertTrue(store.loadConnection(key.connection).sessions.isEmpty())
                assertTrue(store.snapshot(key).entries.isEmpty())
                assertEquals(listOf(other), store.loadConnection("account-b").sessions)
                store.removeChat(ChatKey("account-b", other.id))
                assertTrue(store.loadConnection("account-b").sessions.isEmpty())
            }
        } finally { store.close(); root.deleteRecursively() }
    }

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
    @Test
    fun pagesHistoryWithoutAdvancingLiveStateAndRestoresOnlyRecentEntries() = runBlocking {
        val root = Files.createTempDirectory("tau-paged-store-").toFile()
        val path = root.resolve("transcript.db").path
        var store = TranscriptStore({ path })
        val key = ChatKey("paged", "chat")
        val entries = (0 until 10_000).map { index -> TranscriptEntry("e$index", if (index == 0) null else "e${index - 1}",
            role = EntryRole.User, content = listOf(EntryContent(ContentKind.Text, "Message $index π🧠"))) }
        try {
            val delivered = store.beginSend(key, "Already delivered outside the page", "")
            store.addFiles(key, listOf(PickedFile("kept.txt", "retained file".encodeToByteArray())))
            val uncertain = store.beginSend(key, "Keep uncertain work", "Kept draft")
            store.updateSend(key, uncertain.copy(status = SendStatus.Unconfirmed))
            val cut = TranscriptCut("g", 0, entries.last().id, entries.takeLast(50), QueueState(), "e9949", listOf(delivered.requestId))
            assertTrue(store.applySnapshot(key, cut))
            val chat = store.chat(key)
            val pageKey = chat.pages.single().key
            assertEquals(50, chat.rows.size)
            assertEquals(listOf(uncertain.requestId), chat.pending.map { it.requestId })
            assertEquals("e9949", chat.before)
            assertEquals(null, store.cachedHistory(key, "g", "e9949"))
            val page = HistoryPage(entries.subList(9900, 9950), "e9899")
            assertFalse(store.applyHistory(key, "old", "e9949", page))
            assertFalse(store.applyHistory(key, "g", "wrong", page))
            assertTrue(store.applyHistory(key, "g", "e9949", page))
            assertEquals(100, chat.rows.size)
            assertEquals(pageKey, chat.pages.last().key)
            assertEquals(0L, chat.position.sequence)
            assertEquals("e9949", chat.position.before)
            assertEquals("e9899", chat.before)
            BundledSQLiteDriver().open(path).use { db ->
                db.prepare("CREATE TRIGGER reject_page BEFORE INSERT ON records WHEN NEW.kind='entry' AND NEW.id='e9850' BEGIN SELECT RAISE(ABORT,'page failure'); END").use { it.step() }
                assertFailsWith<Exception> { store.applyHistory(key, "g", "e9899", HistoryPage(entries.subList(9850, 9900), "e9849")) }
                assertEquals(100, chat.rows.size)
                assertEquals("e9899", chat.before)
                db.prepare("DROP TRIGGER reject_page").use { it.step() }
            }
            var end = 9900
            while (end > 0) {
                val start = (end - 50).coerceAtLeast(0)
                assertTrue(store.applyHistory(key, "g", "e${end - 1}", HistoryPage(entries.subList(start, end), if (start == 0) null else "e${start - 1}")))
                end = start
            }
            assertEquals(10_000, chat.rows.size)
            assertEquals(200, chat.pages.size)
            val live = TranscriptEntry("live-stream", entries.last().id, phase = EntryPhase.Live, origin = EntryOrigin(streamId = "stream"),
                role = EntryRole.Assistant, content = listOf(EntryContent(ContentKind.Thinking, "π")))
            assertTrue(store.applyUpdates(key, listOf(TranscriptPatch("g", 1, TranscriptChange.Entry(live)),
                TranscriptPatch("g", 2, TranscriptChange.Delta(live.id, 0, "🧠")))))
            store.trimHistory(key)
            assertEquals(51, chat.byId.size)
            assertEquals(2L, chat.position.sequence)
            assertEquals("π🧠", chat.rows.last().entry.content.single().text)
            store.close(); store = TranscriptStore({ path })
            val restored = store.chat(key)
            assertEquals(51, restored.byId.size)
            assertEquals("e9949", restored.before)
            assertEquals("π🧠", restored.rows.last().entry.content.single().text)
            assertEquals("Kept draft", restored.preferences["draft"])
            assertEquals(uncertain.requestId, restored.pending.single().requestId)
            assertEquals("retained file", store.readFile(key, uncertain.files.single()).bytes.decodeToString())
            val cached = requireNotNull(store.cachedHistory(key, "g", "e9949"))
            assertTrue(store.applyHistory(key, "g", "e9949", cached))
            assertEquals(101, restored.byId.size)
            assertEquals(2L, restored.position.sequence)
            assertTrue(store.applySnapshot(key, cut.copy(generation = "new", savedStreams = listOf("stream"))))
            assertEquals(50, restored.byId.size)
            assertEquals(null, store.cachedHistory(key, "new", "e9949"))
            assertFalse(store.applyHistory(key, "g", "e9949", page))
        } finally { store.close(); root.deleteRecursively() }
    }

}
