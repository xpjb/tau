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

class LocalStoreTest {
    @Test
    fun migratesSessionMetadataAndWritesOnlyChangedRows() = runBlocking {
        val root = Files.createTempDirectory("tau-session-rows-").toFile()
        val path = root.resolve("transcript.db").path
        val key = ChatKey("account-a", "chat")
        val first = SessionSummary(key.session, "Chat 🧠", SessionStatus.Idle, "Ready", SessionModel("provider", "model"), "parent", 1, 2, ContextUsage(64000, 200000))
        val sessions = listOf(first) + (1..200).map { first.copy(id = "other-$it", model = null, parentId = null, contextUsage = null) }
        val other = first.copy(title = "Other account", contextUsage = ContextUsage(null, 128000))
        val oldEntry = """{"id":"saved","content":[{"kind":"text","text":"Old cache"}]}"""
        var store = LocalStore({ path })
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
                        listOf(key.connection, key.session, "entry", "saved", oldEntry),
                        listOf(key.connection, key.session, "position", "current", """{"generation":"g","sequence":4,"head":"saved"}"""),
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
                db.prepare("PRAGMA user_version").use { it.step(); assertEquals(4, it.getInt(0)) }
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
                store = LocalStore({ path })
                assertEquals(StoredConnection(expected, key.session), store.loadConnection(key.connection))
                assertTrue(store.chat(key).rows.isEmpty())
                assertEquals(0L, store.chat(key).position.sequence)
                assertEquals("Retained draft", store.chat(key).preferences["draft"])
                assertTrue(store.readFile(key, store.chat(key).files.single()).bytes.contentEquals(byteArrayOf(1, 2, 3, 4)))
                store.saveSessions(key.connection, emptyList())
                assertTrue(store.loadConnection(key.connection).sessions.isEmpty())
                assertTrue(store.chat(key).rows.isEmpty())
                assertEquals(listOf(other), store.loadConnection("account-b").sessions)
                store.removeChat(ChatKey("account-b", other.id))
                assertTrue(store.loadConnection("account-b").sessions.isEmpty())
            }
        } finally { store.close(); root.deleteRecursively() }
    }

    @Test
    fun applies_flat_pages_deltas_and_queue_state_while_preserving_local_work() = runBlocking {
        val root = Files.createTempDirectory("tau-flat-store-").toFile()
        val path = root.resolve("local.db").path
        var store = LocalStore({ path })
        val key = ChatKey("account", "chat")
        try {
            val events = (0L until 150L).map { order -> TranscriptEvent("s:$order", order, "saved", role = EventRole.Assistant,
                kind = EventKind.Thinking, text = "Thinking $order π🧠") }
            val queue = QueueState(available = true, runId = "run", capabilities = listOf("queue_run_prefix"), boundaries = listOf("turn"))
            val pending = store.beginSend(key, "Same", "")
            val cut = TranscriptCut("g", 0, events.takeLast(50), queue, 100, delivered = listOf(pending.requestId))
            assertTrue(store.applySnapshot(key, cut))
            val chat = store.chat(key)
            assertTrue(chat.pending.isEmpty())
            val row = chat.rows.last()
            assertTrue(store.applyHistory(key, "g", 100, HistoryPage(events.subList(50, 100), 50)))
            assertTrue(store.applyHistory(key, "g", 50, HistoryPage(events.take(50))))
            assertEquals(events, chat.rows.map { it.event })
            assertSame(row, chat.rows.last())
            val live = TranscriptEvent("stream:live:0", 150, "live", phase = EventPhase.Live, origin = EventOrigin(streamId = "live"),
                role = EventRole.Assistant, kind = EventKind.Thinking, text = "Start")
            assertTrue(store.applyUpdates(key, listOf(TranscriptPatch("g", 1, TranscriptChange(events = listOf(live))))))
            val liveRow = chat.rows.last()
            for (index in 2L..101L) assertTrue(store.applyUpdates(key, listOf(TranscriptPatch("g", index, TranscriptChange(delta = TextDelta(live.id, "π"))))))
            assertSame(liveRow, chat.rows.last())
            assertEquals("Start" + "π".repeat(100), liveRow.event.text)
            assertFalse(store.applyUpdates(key, listOf(TranscriptPatch("g", 103, TranscriptChange(delta = TextDelta(live.id, "gap"))))))
            assertFalse(store.applySnapshot(key, cut))
            assertEquals(101L, chat.position.sequence)
            val saved = liveRow.event.copy(entryId = "saved-live", phase = EventPhase.Saved)
            assertTrue(store.applySnapshot(key, TranscriptCut("g", 101, events.takeLast(49) + saved, queue, 101)))
            assertEquals(151, chat.rows.size)
            assertSame(liveRow, chat.rows.last())
            assertEquals(EventPhase.Saved, liveRow.event.phase)
            store.trimHistory(key)
            assertEquals(50, chat.rows.size)
            assertEquals(101L, chat.before)

            val first = store.beginSend(key, "Same", "")
            val second = store.beginSend(key, "Same", "")
            val queued = queue.copy(requests = listOf(QueuedRequest(second.requestId, 0, "steer", "Second"), QueuedRequest(first.requestId, 0, "steer", "First")))
            assertTrue(store.applyUpdates(key, listOf(TranscriptPatch("g", 102, TranscriptChange(queue = queued)))))
            assertEquals(listOf(second.requestId, first.requestId), chat.pending.map { it.requestId })
            store.acknowledge(key.connection, first.requestId, true, disposition = "queued")
            assertTrue(chat.pending.all { it.status == SendStatus.Queued })
            val control = store.beginControl(key, "g") { QueueOperation.Prefix(it.runId, listOf(QueueRef(second.requestId, 0)), "turn") }
            val waiting = queued.copy(control = QueueControl(control.commandId, "run", "prefix", "turn", listOf(QueueRef(second.requestId, 0)), "waiting"))
            assertTrue(store.applyUpdates(key, listOf(TranscriptPatch("g", 103, TranscriptChange(queue = waiting)))))
            assertEquals("waiting", chat.controls.single().status)
            store.addFiles(key, listOf(PickedFile("kept.txt", "bytes".encodeToByteArray())))
            store.setPreference(key, "draft", "Kept draft")
            store.setExpanded(key, "details:${live.id}", true)
            BundledSQLiteDriver().open(path).use { db ->
                db.prepare("SELECT count(*) FROM records WHERE kind IN ('entry','position','page')").use { it.step(); assertEquals(0, it.getInt(0)) }
                db.prepare("CREATE TRIGGER reject_pending BEFORE DELETE ON records WHEN OLD.kind='pending' BEGIN SELECT RAISE(ABORT,'failure'); END").use { it.step() }
                assertFailsWith<Exception> { store.applyUpdates(key, listOf(TranscriptPatch("g", 104, TranscriptChange(delivered = listOf(first.requestId))))) }
                assertEquals(103L, chat.position.sequence)
                assertEquals(2, chat.pending.size)
                db.prepare("DROP TRIGGER reject_pending").use { it.step() }
            }
            val distant = (200L until 250L).map { order -> events.first().copy(id = "s:$order", order = order, text = "Later $order") }
            assertTrue(store.applySnapshot(key, TranscriptCut("g", 104, distant, waiting, 200)))
            assertEquals(distant, chat.rows.map { it.event })
            assertEquals(200L, chat.before)
            store.disconnect(key.connection)
            store.close(); store = LocalStore({ path })
            val restored = store.chat(key)
            assertTrue(restored.rows.isEmpty())
            assertEquals("Kept draft", restored.preferences["draft"])
            assertEquals("true", restored.preferences["expanded:details:${live.id}"])
            assertEquals("bytes", store.readFile(key, restored.files.single()).bytes.decodeToString())
            assertEquals(control.operation, restored.controls.single().operation)
            assertTrue(restored.pending.all { it.status == SendStatus.Unconfirmed })
            assertTrue(store.chat(ChatKey("other-account", "chat")).pending.isEmpty())
        } finally { store.close(); root.deleteRecursively() }
    }
}
