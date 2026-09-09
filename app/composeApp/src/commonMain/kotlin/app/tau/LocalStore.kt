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

class LocalStore(
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
                    check(version <= 4) { "This transcript store needs a newer Tau client" }
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
                    if (version < 4) opened.transaction {
                        opened.execSQL("DELETE FROM records WHERE kind IN ('entry','position','page')")
                        opened.execSQL("PRAGMA user_version=4")
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
        val preferences = db.records(ChatKey(identity, ""), "connection").toMap()
        val missing = mutableListOf<Pair<AttachmentDownloadKey, String>>()
        val downloads = db.prepare("SELECT chat,id,value FROM records WHERE connection=? AND kind='download'").use { statement ->
            statement.bindText(1, identity)
            buildMap {
                while (statement.step()) {
                    val key = AttachmentDownloadKey(statement.getText(0), statement.getText(1))
                    val download = TauJson.decodeFromString<SavedDownload>(statement.getText(2))
                    if (PlatformServices.downloadExists(download)) put(key, download) else missing.add(key to statement.getText(2))
                }
            }
        }
        if (missing.isNotEmpty()) db.transaction {
            for ((key, value) in missing) db.remove(ChatKey(identity, key.sessionId), "download", key.entryId, value)
        }
        StoredConnection(sessions, preferences["selected"], preferences["readAt"]?.let { TauJson.decodeFromString<Map<String, Long>>(it) }.orEmpty(), downloads)
    }

    suspend fun recordDownload(key: ChatKey, entryId: String, download: SavedDownload, available: Boolean = true) = access { db ->
        val value = TauJson.encodeToString(download)
        if (available) db.write(key, "download", listOf(entryId to value))
        else db.remove(key, "download", entryId, value)
    }

    suspend fun saveSessions(identity: String, sessions: List<SessionSummary>, readAt: Map<String, Long>? = null) = access { db ->
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
            if (readAt != null) db.write(ChatKey(identity, ""), "connection", listOf("readAt" to TauJson.encodeToString(readAt)))
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

    suspend fun select(key: ChatKey, readAt: Map<String, Long>? = null) = access { db ->
        db.transaction {
            db.write(ChatKey(key.connection, ""), "connection", listOf("selected" to key.session))
            if (readAt != null) db.write(ChatKey(key.connection, ""), "connection", listOf("readAt" to TauJson.encodeToString(readAt)))
        }
    }

    suspend fun chat(key: ChatKey): RetainedChat = access { db -> loadChat(db, key) }

    private fun loadChat(db: SQLiteConnection, key: ChatKey): RetainedChat = chats.getOrPut(key) {
        RetainedChat(key).also { chat ->
            Snapshot.withMutableSnapshot {
                chat.mutablePending.addAll(db.records(key, "pending").map { TauJson.decodeFromString<PendingSend>(it.second) })
                chat.mutableControls.addAll(db.records(key, "control").map { TauJson.decodeFromString<PendingControl>(it.second) })
                chat.mutablePreferences.putAll(db.records(key, "preference").toMap())
                db.prepare("SELECT id,name,size FROM files WHERE connection=? AND chat=? AND owner='' ORDER BY rowid").use { statement ->
                    statement.bindText(1, key.connection); statement.bindText(2, key.session)
                    while (statement.step()) chat.mutableFiles.add(DraftFile(statement.getText(0), statement.getText(1), statement.getLong(2)))
                }
            }
        }
    }

    suspend fun applySnapshot(key: ChatKey, snapshot: TranscriptCut): Boolean = access { db ->
        val chat = loadChat(db, key)
        if (snapshot.generation == chat.position.generation && snapshot.sequence < chat.position.sequence) return@access false
        require(snapshot.generation.isNotEmpty() && snapshot.sequence >= 0)
        val replaced = snapshot.generation != chat.position.generation
        val connected = !replaced && snapshot.before != null && snapshot.events.any { it.order >= snapshot.before && it.id in chat.byId }
        val older = if (connected) chat.rows.map { it.event }.filter { it.order < snapshot.before && it.phase != EventPhase.Live } else emptyList()
        val events = (older + snapshot.events).associateBy { it.id }.values
        require(snapshot.events.map { it.id }.distinct().size == snapshot.events.size)
        if (!replaced) for (event in snapshot.events) chat.byId[event.id]?.let { require(it.event.order == event.order) }
        require(events.all { it.id.isNotEmpty() && it.order >= 0 } && events.map { it.order }.distinct().size == events.size)
        val controls = reconcileControls(chat.controls, snapshot.queue, replaced)
        val delivered = snapshot.delivered.toMutableSet()
        snapshot.events.filter { it.phase == EventPhase.Saved }.mapNotNullTo(delivered) { it.origin.requestId }
        val pending = reconcilePending(chat.pending, snapshot.queue, delivered, controls, true)
        if (pending != chat.pending || controls != chat.controls) db.transaction { db.replacePending(key, pending, controls) }
        Snapshot.withMutableSnapshot {
            chat.merge(events, replace = true)
            chat.position = ChatPosition(snapshot.generation, snapshot.sequence, snapshot.queue)
            chat.before = if (!connected) snapshot.before else chat.before?.let { minOf(it, snapshot.before) }
            chat.mutablePending.replace(pending); chat.mutableControls.replace(controls)
            chat.synchronized = true
        }
        true
    }

    suspend fun applyHistory(key: ChatKey, generation: String, cursor: Long, page: HistoryPage): Boolean = access { db ->
        val chat = loadChat(db, key)
        if (generation != chat.position.generation || cursor != chat.before) return@access false
        require(page.events.isNotEmpty() && (page.before == null || page.before < cursor))
        require(page.events.all { it.id.isNotEmpty() && it.order in 0 until cursor && (page.before == null || it.order >= page.before) })
        require(page.events.map { it.id }.distinct().size == page.events.size)
        require(page.events.zipWithNext().all { (left, right) -> left.order < right.order })
        for (event in page.events) chat.byId[event.id]?.let { require(it.event.order == event.order) }
        val added = page.events.filter { it.id !in chat.byId }
        require((chat.rows.map { it.event.order } + added.map { it.order }).distinct().size == chat.rows.size + added.size)
        Snapshot.withMutableSnapshot { chat.merge(added); chat.before = page.before }
        true
    }

    suspend fun applyUpdates(key: ChatKey, updates: List<TranscriptPatch>): Boolean = access { db ->
        val chat = loadChat(db, key)
        if (!chat.synchronized) return@access false
        var position = chat.position
        val changed = linkedMapOf<String, TranscriptEvent>()
        val removed = mutableSetOf<String>()
        val delivered = mutableSetOf<String>()
        var valid = true
        for (patch in updates) {
            if (patch.generation == position.generation && patch.sequence <= position.sequence) continue
            if (patch.generation != position.generation || patch.sequence != position.sequence + 1) { valid = false; break }
            val change = patch.change
            try {
                require(change.events.all { it.id.isNotEmpty() && it.order >= 0 })
                require(change.events.map { it.id }.distinct().size == change.events.size)
                require(change.events.map { it.order }.distinct().size == change.events.size)
                val delta = change.delta?.let { delta ->
                    val event = requireNotNull(changed[delta.eventId] ?: chat.byId[delta.eventId]?.event?.takeUnless { delta.eventId in removed })
                    require(event.phase == EventPhase.Live)
                    event.copy(text = event.text + delta.text)
                }
                for (event in change.events) {
                    val prior = changed[event.id] ?: chat.byId[event.id]?.event
                    require(prior == null || prior.order == event.order)
                    val index = chat.rows.binarySearchBy(event.order) { it.event.order }
                    val owner = chat.rows.getOrNull(index)?.key
                    require(owner == null || owner == event.id || owner in removed || owner in change.removed)
                    require(changed.values.none { it.order == event.order && it.id != event.id && it.id !in change.removed })
                }
                for (id in change.removed) { removed.add(id); changed.remove(id) }
                if (delta != null) changed[delta.id] = delta
                for (event in change.events) { changed[event.id] = event; removed.remove(event.id) }
                delivered.addAll(change.delivered)
                position = position.copy(sequence = patch.sequence, queue = change.queue ?: position.queue)
            } catch (_: IllegalArgumentException) { valid = false; break }
        }
        val controls = reconcileControls(chat.controls, position.queue, false)
        val pending = reconcilePending(chat.pending, position.queue, delivered, controls, false)
        if (pending != chat.pending || controls != chat.controls) db.transaction { db.replacePending(key, pending, controls) }
        Snapshot.withMutableSnapshot {
            chat.merge(changed.values, removed)
            chat.position = position; chat.synchronized = valid
            chat.mutablePending.replace(pending); chat.mutableControls.replace(controls)
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
    prepare("INSERT INTO records (connection,chat,kind,id,value) VALUES (?,?,?,?,?) ON CONFLICT(connection,chat,kind,id) DO UPDATE SET value=excluded.value WHERE value IS NOT excluded.value").use { statement ->
        for ((id, value) in values) {
            statement.bindText(1, key.connection); statement.bindText(2, key.session); statement.bindText(3, kind)
            statement.bindText(4, id); statement.bindText(5, value); statement.step(); statement.reset()
        }
    }
}

private fun SQLiteConnection.remove(key: ChatKey, kind: String, id: String? = null, value: String? = null) {
    prepare("DELETE FROM records WHERE connection=? AND chat=? AND kind=?" + (if (id == null) "" else " AND id=?") +
        (if (value == null) "" else " AND value=?")).use { statement ->
        statement.bindText(1, key.connection); statement.bindText(2, key.session); statement.bindText(3, kind)
        if (id != null) statement.bindText(4, id)
        if (value != null) statement.bindText(if (id == null) 4 else 5, value)
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
