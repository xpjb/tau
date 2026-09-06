package app.tau

import androidx.compose.runtime.snapshots.Snapshot
import androidx.sqlite.SQLiteConnection
import androidx.sqlite.SQLiteStatement
import androidx.sqlite.driver.bundled.BundledSQLiteDriver
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString

class TranscriptStore(
    private val path: () -> String,
    private val dispatcher: CoroutineDispatcher = Dispatchers.IO,
) {
    private val gate = Mutex()
    private var connection: SQLiteConnection? = null
    private var closed = false
    private val chats = mutableMapOf<ChatKey, RetainedChat>()

    private suspend fun <T> access(block: (SQLiteConnection) -> T): T = gate.withLock {
        withContext(dispatcher) {
            check(!closed) { "Transcript store is closed" }
            val db = connection ?: BundledSQLiteDriver().open(path()).also { opened ->
                try {
                    opened.execSQL("PRAGMA journal_mode=WAL")
                    opened.execSQL("PRAGMA synchronous=FULL")
                    opened.execSQL("PRAGMA busy_timeout=5000")
                    val version = opened.prepare("PRAGMA user_version").use { it.step(); it.getInt(0) }
                    check(version <= 3) { "This transcript store needs a newer Tau client" }
                    if (version < 2) opened.transaction {
                        opened.execSQL("CREATE TABLE IF NOT EXISTS records (connection TEXT NOT NULL, chat TEXT NOT NULL, kind TEXT NOT NULL, id TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(connection,chat,kind,id))")
                        opened.execSQL("CREATE TABLE IF NOT EXISTS files (connection TEXT NOT NULL, chat TEXT NOT NULL, id TEXT NOT NULL, owner TEXT NOT NULL, name TEXT NOT NULL, size INTEGER NOT NULL, body BLOB NOT NULL, PRIMARY KEY(connection,chat,id))")
                        opened.execSQL("CREATE INDEX IF NOT EXISTS records_order ON records(connection,chat,kind)")
                        opened.execSQL("CREATE INDEX IF NOT EXISTS records_requests ON records(connection,id) WHERE kind IN ('pending','control')")
                        opened.execSQL("""
                            CREATE TABLE sessions (
                                connection TEXT NOT NULL, id TEXT NOT NULL, position INTEGER NOT NULL,
                                title TEXT NOT NULL, status TEXT NOT NULL, detail TEXT,
                                provider TEXT, model_id TEXT, parent_id TEXT,
                                created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL,
                                context_tokens INTEGER, context_window INTEGER,
                                PRIMARY KEY(connection,id)
                            )
                        """.trimIndent())
                        opened.execSQL("CREATE INDEX sessions_order ON sessions(connection,position)")
                        opened.prepare("SELECT connection,value FROM records WHERE chat='' AND kind='connection' AND id='sessions'").use { statement ->
                            while (statement.step()) opened.writeSessions(statement.getText(0), TauJson.decodeFromString(statement.getText(1)))
                        }
                        opened.execSQL("DELETE FROM records WHERE chat='' AND kind='connection' AND id='sessions'")
                        opened.execSQL("PRAGMA user_version=2")
                    }
                    if (version < 3) opened.transaction {
                        opened.execSQL("DELETE FROM records WHERE kind IN ('entry','position','page')")
                        opened.execSQL("PRAGMA user_version=3")
                    }
                    connection = opened
                } catch (error: Throwable) { opened.close(); throw error }
            }
            block(db)
        }
    }

    suspend fun loadConnection(identity: String): StoredConnection = access { db ->
        val sessions = db.prepare("""
            SELECT id,title,status,detail,provider,model_id,parent_id,created_at_ms,updated_at_ms,context_tokens,context_window
            FROM sessions WHERE connection=? ORDER BY position
        """.trimIndent()).use { statement ->
            statement.bindText(1, identity)
            buildList {
                while (statement.step()) add(SessionSummary(
                    id = statement.getText(0), title = statement.getText(1), status = SessionStatus.valueOf(statement.getText(2)),
                    detail = if (statement.isNull(3)) null else statement.getText(3),
                    model = if (statement.isNull(4)) null else SessionModel(statement.getText(4), statement.getText(5)),
                    parentId = if (statement.isNull(6)) null else statement.getText(6),
                    createdAtMs = statement.getLong(7), updatedAtMs = statement.getLong(8),
                    contextUsage = if (statement.isNull(10)) null else ContextUsage(
                        if (statement.isNull(9)) null else statement.getLong(9), statement.getLong(10)),
                ))
            }
        }
        StoredConnection(sessions, db.records(ChatKey(identity, ""), "connection").firstOrNull { it.first == "selected" }?.second)
    }

    suspend fun saveSessions(identity: String, sessions: List<SessionSummary>) = access { db ->
        val ids = sessions.mapTo(mutableSetOf()) { it.id }
        require(ids.size == sessions.size) { "Duplicate session identity" }
        db.transaction {
            val removed = db.prepare("SELECT id FROM sessions WHERE connection=?").use { statement ->
                statement.bindText(1, identity)
                buildList { while (statement.step()) { val id = statement.getText(0); if (id !in ids) add(id) } }
            }
            db.prepare("DELETE FROM sessions WHERE connection=? AND id=?").use { statement ->
                statement.bindText(1, identity)
                for (id in removed) { statement.bindText(2, id); statement.step(); statement.reset() }
            }
            db.writeSessions(identity, sessions)
        }
    }

    suspend fun updateSessionState(identity: String, state: SessionState) = access { db ->
        db.prepare("""
            UPDATE sessions SET status=?1,detail=?2,context_tokens=?3,context_window=?4
            WHERE connection=?5 AND id=?6 AND (status,detail,context_tokens,context_window) IS NOT (?1,?2,?3,?4)
        """.trimIndent()).use { statement ->
            statement.bindText(1, state.status.name); statement.bindTextOrNull(2, state.detail)
            statement.bindLongOrNull(3, state.contextUsage?.tokens); statement.bindLongOrNull(4, state.contextUsage?.contextWindow)
            statement.bindText(5, identity); statement.bindText(6, state.sessionId); statement.step()
        }
    }

    suspend fun select(key: ChatKey) = access { db ->
        db.write(ChatKey(key.connection, ""), "connection", listOf("selected" to key.session))
    }

    suspend fun chat(key: ChatKey): RetainedChat = access { db -> loadChat(db, key) }

    private fun loadChat(db: SQLiteConnection, key: ChatKey): RetainedChat = chats.getOrPut(key) {
        RetainedChat(key).also { chat ->
            chat.position = db.records(key, "position").firstOrNull()?.second?.let { TauJson.decodeFromString(it) } ?: StoredPosition()
            val recent = chat.position.recent
            if (recent.isNotEmpty()) db.prepare("SELECT value FROM records WHERE connection=? AND chat=? AND kind='entry' AND id IN (${recent.joinToString(",") { "?" }})").use { statement ->
                statement.bindText(1, key.connection); statement.bindText(2, key.session)
                recent.forEachIndexed { index, id -> statement.bindText(index + 3, id) }
                val entries = mutableMapOf<String, TranscriptEntry>()
                while (statement.step()) {
                    val entry = TauJson.decodeFromString<TranscriptEntry>(statement.getText(0))
                    entries[entry.id] = entry
                }
                for (id in recent) chat.byId[id] = EntryRow(checkNotNull(entries[id]) { "Recent cached entry is missing" })
            }
            chat.before = chat.position.before
            Snapshot.withMutableSnapshot {
                chat.mutablePending.addAll(db.records(key, "pending").map { TauJson.decodeFromString<PendingSend>(it.second) })
                chat.mutableControls.addAll(db.records(key, "control").map { TauJson.decodeFromString<PendingControl>(it.second) })
                chat.mutablePreferences.putAll(db.records(key, "preference").toMap())
                db.prepare("SELECT id,name,size FROM files WHERE connection=? AND chat=? AND owner='' ORDER BY rowid").use { statement ->
                    statement.bindText(1, key.connection); statement.bindText(2, key.session)
                    while (statement.step()) chat.mutableFiles.add(DraftFile(statement.getText(0), statement.getText(1), statement.getLong(2)))
                }
                chat.rebuildRows()
            }
        }
    }

    suspend fun snapshot(key: ChatKey): TranscriptCut = access { db ->
        val chat = loadChat(db, key)
        TranscriptCut(chat.position.generation, chat.position.sequence, chat.position.head, chat.byId.values.map { it.entry }, chat.queue, chat.before)
    }

    suspend fun applySnapshot(key: ChatKey, snapshot: TranscriptCut): Boolean = access { db ->
        val chat = loadChat(db, key)
        if (snapshot.generation == chat.position.generation && snapshot.sequence < chat.position.sequence) return@access false
        require(snapshot.generation.isNotEmpty() && snapshot.sequence >= 0) { "Invalid transcript position" }
        val entries = snapshot.entries.associateByTo(linkedMapOf()) { it.id }
        require(entries.size == snapshot.entries.size) { "Duplicate transcript entry" }
        require(snapshot.before == null || snapshot.before !in entries) { "Invalid snapshot boundary" }
        var before = snapshot.before
        val replaced = snapshot.generation != chat.position.generation
        if (!replaced) {
            while (before != null) {
                val prior = chat.byId[before]?.entry?.takeIf { it.phase == EntryPhase.Saved } ?: break
                entries[prior.id] = prior
                before = prior.parentId
            }
        }
        val savedStreams = snapshot.savedStreams.toMutableSet()
        savedStreams.addAll(entries.values.filter { it.phase == EntryPhase.Saved }.mapNotNull { it.origin.streamId })
        for (row in chat.byId.values) {
            val prior = row.entry
            if (prior.phase == EntryPhase.Saved || prior.id in entries || prior.origin.streamId in savedStreams) continue
            entries[prior.id] = prior.copy(phase = EntryPhase.Interrupted, parentId = prior.parentId?.takeIf { it in entries })
        }
        val checked = mutableSetOf<String>()
        for (entry in entries.values) {
            require(entry.id.isNotEmpty()) { "Empty transcript identity" }
            val path = mutableSetOf<String>()
            var cursor: String? = entry.id
            while (cursor != null && cursor != before && cursor !in checked) {
                require(path.add(cursor)) { "Cyclic transcript ancestry" }
                val ancestor = requireNotNull(entries[cursor]) { "Missing transcript parent" }
                ancestor.parentId?.let { require(it == before || entries[it]?.phase == EntryPhase.Saved) { "Provisional transcript parent" } }
                cursor = ancestor.parentId
            }
            checked.addAll(path)
        }
        snapshot.head?.let { require(entries[it]?.phase == EntryPhase.Saved) { "Invalid transcript head" } }
        val position = recentPosition(StoredPosition(snapshot.generation, snapshot.sequence, snapshot.head, snapshot.queue), entries::get,
            entries.values.filter { it.phase != EntryPhase.Saved })
        val controls = reconcileControls(chat.controls, snapshot.queue, snapshot.generation != chat.position.generation)
        val delivered = snapshot.delivered.toMutableSet()
        entries.values.mapNotNullTo(delivered) { it.origin.requestId.takeIf { _ -> it.phase == EntryPhase.Saved } }
        val pending = reconcilePending(chat.pending, snapshot.queue, delivered, controls, true)
        val preferences = chat.defaultExpansions(entries.values)
        db.transaction {
            db.write(key, "preference", preferences)
            if (replaced) db.remove(key, "entry")
            else for ((id, row) in chat.byId) if (row.entry.phase != EntryPhase.Saved && id !in entries) db.remove(key, "entry", id)
            db.write(key, "entry", entries.values.filter { replaced || chat.byId[it.id]?.entry != it }.map { it.id to TauJson.encodeToString(it) })
            db.write(key, "position", listOf("current" to TauJson.encodeToString(position)))
            db.replacePending(key, pending, controls)
        }
        val oldByKey = chat.byId.values.associateBy { it.key }
        chat.byId.clear()
        Snapshot.withMutableSnapshot {
            for (entry in entries.values) {
                val row = oldByKey[entry.displayKey] ?: EntryRow(entry)
                row.entry = entry
                chat.byId[entry.id] = row
            }
            chat.position = position
            chat.before = before
            if (replaced) chat.pageStarts.clear()
            if (chat.pageStarts.isEmpty()) snapshot.entries.firstOrNull { it.role != null }?.let { chat.pageStarts.add(it.displayKey) }
            chat.mutablePreferences.putAll(preferences)
            chat.mutablePending.replace(pending)
            chat.mutableControls.replace(controls)
            chat.synchronized = true
            chat.rebuildRows()
        }
        true
    }

    suspend fun cachedHistory(key: ChatKey, generation: String, cursor: String): HistoryPage? = access { db ->
        val chat = loadChat(db, key)
        if (generation != chat.position.generation) return@access null
        var before: String? = cursor
        val entries = mutableListOf<TranscriptEntry>()
        var bytes = 0L
        val seen = mutableSetOf<String>()
        db.prepare("SELECT value FROM records WHERE connection=? AND chat=? AND kind='entry' AND id=?").use { statement ->
            statement.bindText(1, key.connection); statement.bindText(2, key.session)
            while (before != null) {
                require(seen.add(before)) { "Cyclic cached history" }
                statement.bindText(3, before)
                if (!statement.step()) break
                val entry = TauJson.decodeFromString<TranscriptEntry>(statement.getText(0))
                statement.reset()
                require(entry.phase == EntryPhase.Saved) { "Provisional history cursor" }
                if (entries.isNotEmpty() && (entries.size >= HistoryPageEntries || bytes + entry.pageBytes > HistoryPageBytes)) break
                entries.add(entry); bytes += entry.pageBytes; before = entry.parentId
            }
        }
        if (entries.isEmpty()) null else HistoryPage(entries.asReversed(), before)
    }

    suspend fun applyHistory(key: ChatKey, generation: String, cursor: String, page: HistoryPage): Boolean = access { db ->
        val chat = loadChat(db, key)
        if (generation != chat.position.generation || cursor != chat.before) return@access false
        val entries = page.entries.associateByTo(linkedMapOf()) { it.id }
        require(entries.size == page.entries.size && entries.isNotEmpty()) { "Invalid history page" }
        var parent: String? = cursor
        val branch = mutableSetOf<String>()
        while (parent != null && parent != page.before) {
            require(branch.add(parent)) { "Cyclic history page" }
            val entry = requireNotNull(entries[parent]) { "Missing history parent" }
            require(entry.phase == EntryPhase.Saved) { "Provisional history parent" }
            parent = entry.parentId
        }
        require(parent == page.before && cursor in branch) { "Invalid history boundary" }
        for (entry in entries.values) {
            require(entry.id.isNotEmpty() && (entry.id in branch || entry.phase == EntryPhase.Interrupted && entry.parentId in branch)) { "Unrelated history entry" }
            val prior = chat.byId[entry.id]?.entry
            require(prior == null || prior == entry) { "History page changed a retained entry" }
        }
        val preferences = chat.defaultExpansions(entries.values)
        val position = chat.position.copy(recent = (chat.position.recent + entries.values.filter { it.phase == EntryPhase.Interrupted }.map { it.id }).distinct())
        db.transaction {
            db.write(key, "preference", preferences)
            db.write(key, "entry", entries.values.filter { chat.byId[it.id]?.entry != it }.map { it.id to TauJson.encodeToString(it) })
            if (position != chat.position) db.write(key, "position", listOf("current" to TauJson.encodeToString(position)))
        }
        Snapshot.withMutableSnapshot {
            chat.position = position
            for (entry in entries.values) if (entry.id !in chat.byId) chat.byId[entry.id] = EntryRow(entry)
            chat.before = page.before
            page.entries.firstOrNull { it.role != null }?.let { chat.pageStarts.add(it.displayKey) }
            chat.mutablePreferences.putAll(preferences)
            chat.rebuildRows()
        }
        true
    }

    suspend fun trimHistory(key: ChatKey) = access { db ->
        val chat = loadChat(db, key)
        Snapshot.withMutableSnapshot {
            val recent = chat.position.recent.toSet()
            chat.byId.keys.retainAll(recent)
            chat.before = chat.position.before
            chat.pageStarts.clear()
            chat.rebuildRows()
        }
    }

    suspend fun applyUpdates(key: ChatKey, updates: List<TranscriptPatch>): Boolean = access { db ->
        val chat = loadChat(db, key)
        if (!chat.synchronized) return@access false
        var position = chat.position
        val changed = linkedMapOf<String, TranscriptEntry>()
        val removed = mutableSetOf<String>()
        var valid = true
        val delivered = mutableSetOf<String>()
        for (patch in updates) {
            if (patch.generation == position.generation && patch.sequence <= position.sequence) continue
            if (patch.generation != position.generation || patch.sequence != position.sequence + 1) { valid = false; break }
            try {
                when (val change = patch.change) {
                    is TranscriptChange.Entry -> {
                        val entry = change.entry
                        require(entry.id.isNotEmpty() && entry.id != entry.parentId) { "Invalid entry identity" }
                        val prior = changed[entry.id] ?: chat.byId[entry.id]?.entry?.takeUnless { entry.id in removed }
                        require(prior == null || prior.phase == EntryPhase.Live && entry.phase == EntryPhase.Live) { "Replacing a saved entry" }
                        entry.parentId?.let { parent ->
                            require((changed[parent] ?: chat.byId[parent]?.entry?.takeUnless { parent in removed })?.phase == EntryPhase.Saved) { "Missing saved parent" }
                        }
                        if (entry.phase == EntryPhase.Saved) {
                            entry.origin.streamId?.let { stream -> removed.add("live-$stream"); changed.remove("live-$stream") }
                            position = position.copy(head = entry.id)
                            entry.origin.requestId?.let(delivered::add)
                        }
                        changed[entry.id] = entry
                    }
                    is TranscriptChange.Block -> {
                        val entry = requireNotNull(changed[change.entryId] ?: chat.byId[change.entryId]?.entry?.takeUnless { change.entryId in removed })
                        require(entry.phase == EntryPhase.Live && change.index in 0..entry.content.size) { "Invalid transcript block" }
                        val content = entry.content.toMutableList()
                        if (change.index == content.size) content.add(change.content) else content[change.index] = change.content
                        changed[entry.id] = entry.copy(content = content)
                    }
                    is TranscriptChange.Delta -> {
                        val entry = requireNotNull(changed[change.entryId] ?: chat.byId[change.entryId]?.entry?.takeUnless { change.entryId in removed })
                        require(entry.phase == EntryPhase.Live && change.index in entry.content.indices) { "Invalid transcript delta" }
                        val content = entry.content.toMutableList()
                        content[change.index] = content[change.index].copy(text = content[change.index].text + change.delta)
                        changed[entry.id] = entry.copy(content = content)
                    }
                    is TranscriptChange.Head -> {
                        change.head?.let { head -> require((changed[head] ?: chat.byId[head]?.entry?.takeUnless { head in removed })?.phase == EntryPhase.Saved) { "Invalid transcript head" } }
                        position = position.copy(head = change.head)
                    }
                    is TranscriptChange.Queue -> position = position.copy(queue = change.queue)
                    TranscriptChange.Interrupted -> {
                        for (row in chat.byId.values) {
                            val entry = changed[row.entry.id] ?: row.entry
                            if (entry.phase == EntryPhase.Live && entry.id !in removed) changed[entry.id] = entry.copy(phase = EntryPhase.Interrupted)
                        }
                        for ((id, entry) in changed.toMap()) if (entry.phase == EntryPhase.Live) changed[id] = entry.copy(phase = EntryPhase.Interrupted)
                        position = position.copy(queue = position.queue.copy(available = false))
                    }
                }
                position = position.copy(sequence = patch.sequence)
            } catch (_: IllegalArgumentException) { valid = false; break }
        }
        if (position.sequence == chat.position.sequence) { chat.synchronized = valid; return@access valid }
        val membershipChanged = position.head != chat.position.head || removed.isNotEmpty() || changed.values.any { it.id !in chat.byId }
        if (membershipChanged) {
            val provisional = (chat.byId.values.map { changed[it.entry.id] ?: it.entry } + changed.values)
                .filter { it.phase != EntryPhase.Saved && it.id !in removed }.distinctBy { it.id }
            position = recentPosition(position, { id -> changed[id] ?: chat.byId[id]?.entry?.takeUnless { id in removed } }, provisional)
        }
        val queueChanged = position.queue != chat.queue
        val controls = if (queueChanged) reconcileControls(chat.controls, position.queue, false) else chat.controls
        val pending = if (queueChanged || delivered.isNotEmpty()) reconcilePending(chat.pending, position.queue, delivered, controls, false) else chat.pending
        val preferences = chat.defaultExpansions(changed.values)
        db.transaction {
            db.write(key, "preference", preferences)
            for (id in removed) db.remove(key, "entry", id)
            db.write(key, "entry", changed.values.map { it.id to TauJson.encodeToString(it) })
            db.write(key, "position", listOf("current" to TauJson.encodeToString(position)))
            if (pending != chat.pending || controls != chat.controls) db.replacePending(key, pending, controls)
        }
        val oldHead = chat.position.head
        val newRows = mutableListOf<EntryRow>()
        Snapshot.withMutableSnapshot {
            val removedByKey = mutableMapOf<String, EntryRow>()
            for (id in removed) {
                val row = chat.byId.remove(id) ?: continue
                removedByKey[row.key] = row
                if (chat.visibleKeys.remove(row.key)) chat.mutableRows.removeAt(chat.mutableRows.lastIndexOf(row))
            }
            for (entry in changed.values) {
                val prior = chat.byId[entry.id]
                val row = prior ?: removedByKey[entry.displayKey] ?: EntryRow(entry)
                row.entry = entry
                chat.byId[entry.id] = row
                if (prior == null) newRows.add(row)
            }
            chat.position = position
            chat.mutablePreferences.putAll(preferences)
            chat.mutablePending.replace(pending)
            chat.mutableControls.replace(controls)
            chat.synchronized = valid
            val extension = mutableListOf<EntryRow>()
            var cursor = position.head
            while (cursor != null && cursor != oldHead && cursor != chat.before) {
                val row = checkNotNull(chat.byId[cursor])
                extension.add(row)
                cursor = row.entry.parentId
            }
            if (cursor != oldHead || newRows.any { it.entry.phase != EntryPhase.Saved && it.entry.parentId != position.head }) {
                chat.rebuildRows()
            } else {
                if (membershipChanged) {
                    val lastPage = chat.pages.lastOrNull()
                    var count = lastPage?.rows?.size ?: 0
                    var bytes = lastPage?.rows?.sumOf { it.entry.pageBytes } ?: 0L
                    for (row in extension.asReversed() + newRows.filter { it.entry.phase != EntryPhase.Saved }) {
                        if (row.entry.role == null || lastPage?.rows?.contains(row) == true) continue
                        if (count > 0 && (count >= HistoryPageEntries || bytes + row.entry.pageBytes > HistoryPageBytes)) {
                            chat.pageStarts.add(row.key); count = 0; bytes = 0
                        }
                        count++; bytes += row.entry.pageBytes
                    }
                }
                for (row in extension.asReversed()) {
                    chat.branch.add(row.entry.id)
                    if (row.entry.role != null && chat.visibleKeys.add(row.key)) chat.mutableRows.add(row)
                }
                for (row in newRows) {
                    val entry = row.entry
                    if (entry.phase != EntryPhase.Saved && (entry.parentId == null || entry.parentId in chat.branch) && entry.role != null && chat.visibleKeys.add(row.key)) chat.mutableRows.add(row)
                }
                if (membershipChanged) chat.rebuildPages()
            }
        }
        valid
    }

    suspend fun setPreference(key: ChatKey, name: String, value: String) = access { db ->
        val chat = loadChat(db, key)
        db.write(key, "preference", listOf(name to value))
        chat.mutablePreferences[name] = value
    }

    suspend fun setExpanded(key: ChatKey, name: String, expanded: Boolean) = access { db ->
        val chat = loadChat(db, key)
        val preferences = mutableListOf("expanded:$name" to expanded.toString())
        if (name.startsWith("details:")) preferences.add("detailsDefault" to expanded.toString())
        db.transaction { db.write(key, "preference", preferences) }
        Snapshot.withMutableSnapshot { chat.mutablePreferences.putAll(preferences) }
    }

    suspend fun addFiles(key: ChatKey, files: List<PickedFile>) = access { db ->
        val chat = loadChat(db, key)
        require(files.all { it.bytes.isNotEmpty() } && files.size + chat.files.size <= MaxUploadFiles)
        require(files.sumOf { it.bytes.size.toLong() } + chat.files.sumOf { it.size } <= MaxUploadBytes)
        val added = files.map { DraftFile(newRequestId(), it.name, it.bytes.size.toLong()) }
        db.transaction {
            db.prepare("INSERT INTO files (connection,chat,id,owner,name,size,body) VALUES (?,?,?,'',?,?,?)").use { statement ->
                for ((index, file) in files.withIndex()) {
                    statement.bindText(1, key.connection); statement.bindText(2, key.session); statement.bindText(3, added[index].id)
                    statement.bindText(4, file.name); statement.bindLong(5, file.bytes.size.toLong()); statement.bindBlob(6, file.bytes)
                    statement.step(); statement.reset()
                }
            }
        }
        chat.mutableFiles.addAll(added)
    }

    suspend fun removeFile(key: ChatKey, id: String) = access { db ->
        val chat = loadChat(db, key)
        db.prepare("DELETE FROM files WHERE connection=? AND chat=? AND id=? AND owner=''").use { statement ->
            statement.bindText(1, key.connection); statement.bindText(2, key.session); statement.bindText(3, id); statement.step()
        }
        chat.mutableFiles.removeAll { it.id == id }
    }

    suspend fun readFile(key: ChatKey, file: DraftFile): PickedFile = access { db ->
        db.prepare("SELECT body FROM files WHERE connection=? AND chat=? AND id=?").use { statement ->
            statement.bindText(1, key.connection); statement.bindText(2, key.session); statement.bindText(3, file.id)
            check(statement.step()) { "Retained attachment is unavailable" }
            PickedFile(file.name, statement.getBlob(0))
        }
    }

    suspend fun beginSend(key: ChatKey, text: String, draft: String): PendingSend = access { db ->
        val chat = loadChat(db, key)
        val files = chat.files.toList()
        require(text.isNotBlank() || files.isNotEmpty())
        val pending = PendingSend(newRequestId(), text, if (files.isEmpty()) text else null, files,
            status = if (files.isEmpty()) SendStatus.Sending else SendStatus.Preparing)
        db.transaction {
            db.write(key, "pending", listOf(pending.requestId to TauJson.encodeToString(pending)))
            db.write(key, "preference", listOf("draft" to draft))
            db.prepare("UPDATE files SET owner=? WHERE connection=? AND chat=? AND owner=''").use { statement ->
                statement.bindText(1, pending.requestId); statement.bindText(2, key.connection); statement.bindText(3, key.session); statement.step()
            }
        }
        Snapshot.withMutableSnapshot {
            chat.mutablePending.add(pending)
            chat.mutablePreferences["draft"] = draft
            chat.mutableFiles.clear()
        }
        pending
    }

    suspend fun updateSend(key: ChatKey, pending: PendingSend) = access { db ->
        val chat = loadChat(db, key)
        val index = chat.pending.indexOfFirst { it.requestId == pending.requestId }
        if (index < 0) return@access
        db.write(key, "pending", listOf(pending.requestId to TauJson.encodeToString(pending)))
        chat.mutablePending[index] = pending
    }

    suspend fun restoreSend(key: ChatKey, id: String) = access { db ->
        val chat = loadChat(db, key)
        val pending = chat.pending.firstOrNull { it.requestId == id } ?: return@access
        require(pending.status == SendStatus.Rejected) { "Only a definitely unsent message can be restored" }
        require(chat.files.size + pending.files.size <= MaxUploadFiles)
        require(chat.files.sumOf { it.size } + pending.files.sumOf { it.size } <= MaxUploadBytes)
        val draft = listOf(pending.text, chat.preferences["draft"].orEmpty()).filter { it.isNotBlank() }.joinToString("\n\n")
        db.transaction {
            db.remove(key, "pending", id)
            db.write(key, "preference", listOf("draft" to draft))
            db.prepare("UPDATE files SET owner='' WHERE connection=? AND chat=? AND owner=?").use { statement ->
                statement.bindText(1, key.connection); statement.bindText(2, key.session); statement.bindText(3, id); statement.step()
            }
        }
        Snapshot.withMutableSnapshot {
            chat.mutablePending.removeAll { it.requestId == id }
            chat.mutableFiles.addAll(pending.files)
            chat.mutablePreferences["draft"] = draft
        }
    }

    suspend fun beginControl(key: ChatKey, generation: String, operation: (QueueState) -> QueueOperation): PendingControl = access { db ->
        val chat = loadChat(db, key)
        check(chat.synchronized && chat.queue.available && chat.position.generation == generation) { "Synchronize this chat before changing its queue" }
        val control = PendingControl(newRequestId(), chat.position.generation, operation(chat.queue))
        db.write(key, "control", listOf(control.commandId to TauJson.encodeToString(control)))
        chat.mutableControls.add(control)
        control
    }

    suspend fun acknowledge(identity: String, id: String, ok: Boolean, uncertain: Boolean = false, disposition: String? = null, outcome: String? = null, detail: String? = null) = access { db ->
        val matches = mutableListOf<ChatKey>()
        db.prepare("SELECT DISTINCT chat FROM records WHERE connection=? AND id=? AND kind IN ('pending','control')").use { statement ->
            statement.bindText(1, identity); statement.bindText(2, id)
            while (statement.step()) matches.add(ChatKey(identity, statement.getText(0)))
        }
        for (key in matches) {
            val chat = loadChat(db, key)
            var pending = chat.pending.mapNotNull { message ->
                if (message.requestId != id) message
                else if (ok && disposition == "handled") null
                else if (message.status == SendStatus.Queued && chat.synchronized && chat.queue.available && chat.queue.requests.any { it.requestId == id }) message
                else message.copy(status = when { uncertain -> SendStatus.Unconfirmed; !ok -> SendStatus.Rejected; else -> SendStatus.Accepted }, detail = detail)
            }
            val controls = chat.controls.map { control ->
                if (control.commandId != id) control
                else if (chat.queue.control?.commandId == id) control.copy(status = chat.queue.control!!.status, detail = chat.queue.control!!.detail)
                else control.copy(status = if (uncertain) "unconfirmed" else if (ok) outcome ?: "accepted" else "failed", detail = detail)
            }
            if (ok && outcome == "deleted") {
                val deleted = controls.firstOrNull { it.commandId == id }?.operation as? QueueOperation.Delete
                pending = pending.filterNot { it.requestId == deleted?.requestId && it.revision == deleted.revision }
            }
            db.transaction { db.replacePending(key, pending, controls) }
            Snapshot.withMutableSnapshot { chat.mutablePending.replace(pending); chat.mutableControls.replace(controls) }
        }
    }

    suspend fun disconnect(identity: String) = access { db ->
        val pending = mutableMapOf<ChatKey, MutableList<PendingSend>>()
        val controls = mutableMapOf<ChatKey, MutableList<PendingControl>>()
        db.prepare("SELECT chat,kind,value FROM records WHERE connection=? AND kind IN ('pending','control') ORDER BY rowid").use { statement ->
            statement.bindText(1, identity)
            while (statement.step()) {
                val key = ChatKey(identity, statement.getText(0))
                if (statement.getText(1) == "pending") {
                    val record = TauJson.decodeFromString<PendingSend>(statement.getText(2))
                    val interrupted = when (record.status) {
                        SendStatus.Preparing -> record.copy(status = SendStatus.Rejected, detail = "Attachment preparation stopped before sending")
                        SendStatus.Rejected -> record
                        else -> record.copy(status = SendStatus.Unconfirmed)
                    }
                    pending.getOrPut(key) { mutableListOf() }.add(interrupted)
                } else {
                    val record = TauJson.decodeFromString<PendingControl>(statement.getText(2))
                    controls.getOrPut(key) { mutableListOf() }.add(if (record.status in setOf("sending", "accepted", "waiting", "applying")) record.copy(status = "unconfirmed") else record)
                }
            }
        }
        db.transaction {
            for (key in pending.keys + controls.keys) db.replacePending(key, pending[key].orEmpty(), controls[key].orEmpty())
        }
        Snapshot.withMutableSnapshot {
            for ((key, chat) in chats) if (key.connection == identity) {
                chat.synchronized = false
                chat.mutablePending.replace(pending[key].orEmpty())
                chat.mutableControls.replace(controls[key].orEmpty())
            }
        }
    }

    suspend fun invalidate(identity: String, sessionId: String? = null) = access {
        Snapshot.withMutableSnapshot {
            for ((key, chat) in chats) if (key.connection == identity && (sessionId == null || key.session == sessionId)) chat.synchronized = false
        }
    }

    suspend fun dismissPending(key: ChatKey, id: String) = access { db ->
        val chat = loadChat(db, key)
        val record = chat.pending.firstOrNull { it.requestId == id } ?: return@access
        require(record.status == SendStatus.Unconfirmed || record.status == SendStatus.Rejected) { "Use Delete to change Pi's queue" }
        val pending = chat.pending.filterNot { it.requestId == id }
        db.transaction { db.replacePending(key, pending, chat.controls) }
        chat.mutablePending.replace(pending)
    }

    suspend fun removeChat(key: ChatKey) = access { db ->
        db.transaction {
            for (table in listOf("records", "files")) db.prepare("DELETE FROM $table WHERE connection=? AND chat=?").use { statement ->
                statement.bindText(1, key.connection); statement.bindText(2, key.session); statement.step()
            }
            db.prepare("DELETE FROM sessions WHERE connection=? AND id=?").use { statement ->
                statement.bindText(1, key.connection); statement.bindText(2, key.session); statement.step()
            }
        }
        chats.remove(key)
    }

    suspend fun close() = withContext(dispatcher) {
        gate.withLock { closed = true; connection?.close(); connection = null; chats.clear() }
    }
}

private fun recentPosition(position: StoredPosition, entry: (String) -> TranscriptEntry?, provisional: Collection<TranscriptEntry>): StoredPosition {
    val ids = mutableListOf<String>()
    var cursor = position.head
    var bytes = 0L
    while (cursor != null) {
        val row = entry(cursor) ?: break
        if (ids.isNotEmpty() && (ids.size >= HistoryPageEntries || bytes + row.pageBytes > HistoryPageBytes)) break
        ids.add(row.id); bytes += row.pageBytes; cursor = row.parentId
    }
    return position.copy(before = cursor, recent = ids.asReversed() + provisional.map { it.id })
}

private fun reconcilePending(previous: List<PendingSend>, queue: QueueState, delivered: Set<String>, controls: List<PendingControl>, snapshot: Boolean): List<PendingSend> {
    val pending = previous.associateByTo(linkedMapOf()) { it.requestId }
    val deleted = controls.filter { it.status == "deleted" }.mapNotNull { it.operation as? QueueOperation.Delete }.mapTo(mutableSetOf()) { QueueRef(it.requestId, it.revision) }
    val queued = queue.requests.mapTo(mutableSetOf()) { it.requestId }
    for (request in queue.requests) {
        if (QueueRef(request.requestId, request.revision) in deleted || request.requestId in delivered) continue
        val prior = pending[request.requestId]
        pending[request.requestId] = PendingSend(request.requestId, request.text, prior?.wireText, prior?.files.orEmpty(), request.revision,
            if (queue.available) SendStatus.Queued else SendStatus.Unconfirmed)
    }
    for ((id, record) in pending.toMap()) {
        if (id in delivered) pending.remove(id)
        else if (id !in queued && (snapshot && record.status == SendStatus.Accepted || record.status == SendStatus.Queued)) pending[id] = record.copy(status = SendStatus.Unconfirmed)
    }
    return queue.requests.mapNotNull { pending.remove(it.requestId) } + pending.values
}

private fun reconcileControls(previous: List<PendingControl>, queue: QueueState, replaced: Boolean): List<PendingControl> = previous.map { record ->
    val control = queue.control
    if (control?.commandId == record.commandId) record.copy(status = control.status, detail = control.detail)
    else if (replaced && record.status in setOf("sending", "accepted", "waiting", "applying")) record.copy(status = "unconfirmed")
    else record
}

private fun SQLiteConnection.writeSessions(identity: String, sessions: List<SessionSummary>) {
    prepare("""
        INSERT INTO sessions (connection,id,position,title,status,detail,provider,model_id,parent_id,created_at_ms,updated_at_ms,context_tokens,context_window)
        VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)
        ON CONFLICT(connection,id) DO UPDATE SET
            position=excluded.position,title=excluded.title,status=excluded.status,detail=excluded.detail,
            provider=excluded.provider,model_id=excluded.model_id,parent_id=excluded.parent_id,
            created_at_ms=excluded.created_at_ms,updated_at_ms=excluded.updated_at_ms,
            context_tokens=excluded.context_tokens,context_window=excluded.context_window
        WHERE (position,title,status,detail,provider,model_id,parent_id,created_at_ms,updated_at_ms,context_tokens,context_window)
            IS NOT (excluded.position,excluded.title,excluded.status,excluded.detail,excluded.provider,excluded.model_id,excluded.parent_id,
                excluded.created_at_ms,excluded.updated_at_ms,excluded.context_tokens,excluded.context_window)
    """.trimIndent()).use { statement ->
        for ((index, session) in sessions.withIndex()) {
            statement.bindText(1, identity); statement.bindText(2, session.id); statement.bindInt(3, index)
            statement.bindText(4, session.title); statement.bindText(5, session.status.name); statement.bindTextOrNull(6, session.detail)
            statement.bindTextOrNull(7, session.model?.provider); statement.bindTextOrNull(8, session.model?.modelId); statement.bindTextOrNull(9, session.parentId)
            statement.bindLong(10, session.createdAtMs); statement.bindLong(11, session.updatedAtMs)
            statement.bindLongOrNull(12, session.contextUsage?.tokens); statement.bindLongOrNull(13, session.contextUsage?.contextWindow)
            statement.step(); statement.reset()
        }
    }
}

private fun SQLiteStatement.bindTextOrNull(index: Int, value: String?) { if (value == null) bindNull(index) else bindText(index, value) }
private fun SQLiteStatement.bindLongOrNull(index: Int, value: Long?) { if (value == null) bindNull(index) else bindLong(index, value) }

private fun SQLiteConnection.execSQL(sql: String) { prepare(sql).use { it.step() } }

private fun <T> SQLiteConnection.transaction(block: () -> T): T {
    execSQL("BEGIN IMMEDIATE")
    try {
        val value = block()
        execSQL("COMMIT")
        return value
    } catch (error: Throwable) {
        try { execSQL("ROLLBACK") } catch (rollback: Throwable) { error.addSuppressed(rollback) }
        throw error
    }
}

private fun SQLiteConnection.records(key: ChatKey, kind: String): List<Pair<String, String>> =
    prepare("SELECT id,value FROM records WHERE connection=? AND chat=? AND kind=? ORDER BY rowid").use { statement ->
        statement.bindText(1, key.connection); statement.bindText(2, key.session); statement.bindText(3, kind)
        buildList { while (statement.step()) add(statement.getText(0) to statement.getText(1)) }
    }

private fun SQLiteConnection.write(key: ChatKey, kind: String, values: List<Pair<String, String>>) {
    if (values.isEmpty()) return
    prepare("INSERT INTO records (connection,chat,kind,id,value) VALUES (?,?,?,?,?) ON CONFLICT(connection,chat,kind,id) DO UPDATE SET value=excluded.value").use { statement ->
        for ((id, value) in values) {
            statement.bindText(1, key.connection); statement.bindText(2, key.session); statement.bindText(3, kind)
            statement.bindText(4, id); statement.bindText(5, value); statement.step(); statement.reset()
        }
    }
}

private fun SQLiteConnection.remove(key: ChatKey, kind: String, id: String? = null) {
    prepare("DELETE FROM records WHERE connection=? AND chat=? AND kind=?" + if (id == null) "" else " AND id=?").use { statement ->
        statement.bindText(1, key.connection); statement.bindText(2, key.session); statement.bindText(3, kind)
        if (id != null) statement.bindText(4, id)
        statement.step()
    }
}

private fun SQLiteConnection.replacePending(key: ChatKey, pending: List<PendingSend>, controls: List<PendingControl>) {
    remove(key, "pending"); remove(key, "control")
    write(key, "pending", pending.map { it.requestId to TauJson.encodeToString(it) })
    write(key, "control", controls.map { it.commandId to TauJson.encodeToString(it) })
    prepare("DELETE FROM files WHERE connection=? AND chat=? AND owner<>'' AND owner NOT IN (SELECT id FROM records WHERE connection=? AND chat=? AND kind='pending')").use { statement ->
        statement.bindText(1, key.connection); statement.bindText(2, key.session)
        statement.bindText(3, key.connection); statement.bindText(4, key.session); statement.step()
    }
}

private fun <T> MutableList<T>.replace(values: List<T>) { if (this != values) { clear(); addAll(values) } }

private fun RetainedChat.defaultExpansions(entries: Collection<TranscriptEntry>): List<Pair<String, String>> {
    if (preferences["detailsDefault"] != "true") return emptyList()
    return entries.mapNotNull { entry ->
        val key = "expanded:details:${entry.displayKey}"
        if ((entry.role == EntryRole.Assistant || entry.role == EntryRole.Tool) && entry.id !in byId &&
            entry.origin.streamId?.let { "live-$it" } !in byId && key !in preferences) key to "true" else null
    }
}
