package app.tau

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.serialization.encodeToString
import kotlin.time.TimeSource

private const val ReconnectDelayMillis = 2_000L
private const val CommandLoadMillis = 10_000L
private const val DownloadProgressIntervalMillis = 200L

enum class ConnectionStatus { NotConfigured, Connecting, Connected, Offline }

data class SessionExtensionUi(val sessionId: String, val request: ExtensionUiRequest)
data class ExtensionWidget(val lines: List<String>, val placement: String?)
data class AttachmentDownloadKey(val sessionId: String, val entryId: String)

enum class AttachmentDownloadStatus { Downloading, Downloaded, Failed }

data class AttachmentDownload(
    val status: AttachmentDownloadStatus,
    val transferredBytes: Long,
    val totalBytes: Long?,
    val bytesPerSecond: Long? = null,
    val saved: SavedDownload? = null,
    val error: String? = null,
)

data class TauUiState(
    val settings: ConnectionSettings = ConnectionSettings(),
    val editingSettings: Boolean = false,
    val restoring: Boolean = true,
    val connectionStatus: ConnectionStatus = ConnectionStatus.NotConfigured,
    val daemonVersion: String? = null,
    val sessions: List<SessionSummary> = emptyList(),
    val selectedSessionId: String? = null,
    val focusComposerSessionId: String? = null,
    val transcripts: Map<String, RetainedChat> = emptyMap(),
    val drafts: Map<String, String> = emptyMap(),
    val slashCommands: Map<String, List<SlashCommand>> = emptyMap(),
    val loadingCommands: Set<String> = emptySet(),
    val loadingHistory: Set<String> = emptySet(),
    val extensionDialogs: List<SessionExtensionUi> = emptyList(),
    val extensionStatuses: Map<String, Map<String, String>> = emptyMap(),
    val extensionWidgets: Map<String, Map<String, ExtensionWidget>> = emptyMap(),
    val attachmentDownloads: Map<AttachmentDownloadKey, AttachmentDownload> = emptyMap(),
    val pickingFiles: Boolean = false,
    val uploadingSessions: Set<String> = emptySet(),
    val mobileChatVisible: Boolean = false,
    val notice: String? = null,
    val error: String? = null,
)

class TauController(
    dispatcher: CoroutineDispatcher,
    private val store: TranscriptStore = TranscriptStore({ PlatformServices.transcriptDatabasePath }),
) {
    private val scope = CoroutineScope(SupervisorJob() + dispatcher)
    private val client = TauClient()
    private val mutableState = MutableStateFlow(TauUiState())
    private val pending = mutableMapOf<String, PendingAction>()
    private val syncing = mutableSetOf<String>()
    private val downloadJobs = mutableMapOf<AttachmentDownloadKey, Job>()
    private var connectionJob: Job? = null
    private var closeJob: Job? = null
    private var connectionVersion = 0L
    private var socketId: Long? = null
    private var started = false

    val state: StateFlow<TauUiState> = mutableState.asStateFlow()

    fun start(settings: ConnectionSettings? = null) {
        if (started) return
        started = true
        if (settings != null) connect(settings)
        else launch { connect(withContext(Dispatchers.IO) { PlatformServices.loadConnection() }) }
    }

    fun saveConnection(serverUrl: String, token: String) {
        val settings = ConnectionSettings(serverUrl.trim().trimEnd('/'), token.trim())
        if ((!settings.serverUrl.startsWith("http://") && !settings.serverUrl.startsWith("https://")) || settings.token.isBlank()) {
            mutableState.update { it.copy(error = "Enter an HTTP server URL and token.") }
            return
        }
        launch {
            withContext(Dispatchers.IO) { PlatformServices.saveConnection(settings) }
            connect(settings)
        }
    }

    fun showSettings() { mutableState.update { it.copy(editingSettings = true) } }
    fun hideSettings() { if (state.value.settings.token.isNotBlank()) mutableState.update { it.copy(editingSettings = false) } }
    fun showSessionList() { mutableState.update { it.copy(mobileChatVisible = false) } }
    fun dismissError() { mutableState.update { it.copy(error = null) } }
    fun dismissNotice() { mutableState.update { it.copy(notice = null) } }

    fun createSession() { send(CreateSession(newRequestId()), PendingAction.Create) }
    fun renameSession(sessionId: String, title: String) { send(RenameSession(newRequestId(), sessionId, title)) }
    fun deleteSession(sessionId: String) { send(DeleteSession(newRequestId(), sessionId), PendingAction.Delete(sessionId)) }
    fun abort() { state.value.selectedSessionId?.let { send(Abort(newRequestId(), it)) } }
    fun fork(entryId: String) { state.value.selectedSessionId?.let { send(ForkSession(newRequestId(), it, entryId), PendingAction.Select) } }

    fun selectSession(sessionId: String) {
        val previous = state.value.selectedSessionId
        mutableState.update { it.copy(selectedSessionId = sessionId, mobileChatVisible = true, error = null, loadingHistory = emptySet()) }
        syncing.retainAll(setOf(sessionId))
        pending.entries.removeAll { (_, action) -> action is PendingAction.Open && action.sessionId != sessionId || action is PendingAction.History }
        val key = ChatKey(state.value.settings.identity, sessionId)
        launch {
            if (previous != null) { store.invalidate(key.connection, previous); store.trimHistory(ChatKey(key.connection, previous)) }
            loadChat(key)
            store.invalidate(key.connection, sessionId)
            if (state.value.settings.identity == key.connection && state.value.selectedSessionId == sessionId) {
                store.select(key)
                openSession(sessionId)
                loadCommands(sessionId)
            }
        }
    }

    fun consumeComposerFocus(sessionId: String) {
        mutableState.update { if (it.focusComposerSessionId == sessionId) it.copy(focusComposerSessionId = null) else it }
    }

    fun setDraft(sessionId: String, draft: String) {
        val key = ChatKey(state.value.settings.identity, sessionId)
        mutableState.update { it.copy(drafts = it.drafts + (sessionId to draft)) }
        launch { withContext(NonCancellable) { store.setPreference(key, "draft", draft) } }
        loadCommands(sessionId)
    }

    private fun loadCommands(sessionId: String) {
        val current = state.value
        if (current.drafts[sessionId]?.startsWith('/') == true && current.connectionStatus == ConnectionStatus.Connected &&
            sessionId !in current.slashCommands && sessionId !in current.loadingCommands) {
            mutableState.update { it.copy(loadingCommands = it.loadingCommands + sessionId) }
            send(GetCommands(newRequestId(), sessionId), PendingAction.Commands(sessionId))
        }
    }

    fun loadOlder(sessionId: String) {
        val current = state.value
        val chat = current.transcripts[sessionId] ?: return
        val cursor = chat.before ?: return
        if (current.selectedSessionId != sessionId || sessionId in current.loadingHistory) return
        val request = GetHistory(newRequestId(), sessionId, chat.position.generation, cursor)
        val action = PendingAction.History(sessionId, request.generation, cursor)
        pending[request.id] = action
        mutableState.update { it.copy(loadingHistory = it.loadingHistory + sessionId, error = null) }
        launch {
            var sent = false
            try {
                val cached = store.cachedHistory(chat.key, request.generation, cursor)
                if (pending[request.id] != action) return@launch
                if (cached != null) {
                    store.applyHistory(chat.key, request.generation, cursor, cached)
                } else if (socketId != null && chat.synchronized) {
                    send(request, action)
                    sent = true
                    return@launch
                } else {
                    error("This older history is not cached")
                }
            } finally {
                if (!sent && pending[request.id] == action) {
                    pending.remove(request.id)
                    mutableState.update { it.copy(loadingHistory = it.loadingHistory - sessionId) }
                }
            }
        }
    }

    fun setExpanded(sessionId: String, key: String, expanded: Boolean) {
        val chat = ChatKey(state.value.settings.identity, sessionId)
        launch { store.setExpanded(chat, key, expanded) }
    }

    fun saveScroll(sessionId: String, position: ScrollPosition) {
        val key = ChatKey(state.value.settings.identity, sessionId)
        launch { store.setPreference(key, "scroll", TauJson.encodeToString(position)) }
    }

    fun pickFiles() { loadAttachments(PlatformServices::pickFiles) }
    fun attachDroppedFiles(fileUris: List<String>) { if (fileUris.isNotEmpty()) loadAttachments { PlatformServices.readDroppedFiles(fileUris) } }
    fun attachClipboardImage(load: suspend () -> PickedFile) { loadAttachments { listOf(load()) } }

    private fun loadAttachments(load: suspend () -> List<PickedFile>) {
        val current = state.value
        val sessionId = current.selectedSessionId ?: return
        if (current.pickingFiles || sessionId in current.uploadingSessions) return
        val key = ChatKey(current.settings.identity, sessionId)
        val version = connectionVersion
        mutableState.update { it.copy(pickingFiles = true, error = null) }
        launch {
            try { val files = load(); if (files.isNotEmpty()) store.addFiles(key, files) }
            finally { if (version == connectionVersion) mutableState.update { it.copy(pickingFiles = false) } }
        }
    }

    fun removeAttachment(sessionId: String, index: Int) {
        val chat = state.value.transcripts[sessionId] ?: return
        val file = chat.files.getOrNull(index) ?: return
        launch { store.removeFile(chat.key, file.id) }
    }

    fun sendPrompt() {
        val current = state.value
        val sessionId = current.selectedSessionId ?: return
        val chat = current.transcripts[sessionId] ?: return
        val connectionId = socketId ?: return
        val text = current.drafts[sessionId].orEmpty()
        if (current.connectionStatus != ConnectionStatus.Connected || sessionId in current.uploadingSessions || text.isBlank() && chat.files.isEmpty()) return
        val introduction = text.ifBlank { if (chat.files.size == 1) "Please inspect the attached file." else "Please inspect the attached files." }
        val version = connectionVersion
        mutableState.update { it.copy(uploadingSessions = it.uploadingSessions + sessionId, error = null) }
        launch {
            var outgoing: PendingSend? = null
            var attempted = false
            try {
                outgoing = store.beginSend(chat.key, introduction, "")
                mutableState.update { ui ->
                    if (version == connectionVersion && ui.drafts[sessionId].orEmpty() == text) ui.copy(drafts = ui.drafts + (sessionId to "")) else ui
                }
                val uploaded = outgoing.files.map { file -> client.uploadFile(current.settings, sessionId, store.readFile(chat.key, file)) }
                check(version == connectionVersion && socketId == connectionId && state.value.connectionStatus == ConnectionStatus.Connected) { "Connection changed before the message was sent" }
                val message = if (uploaded.isEmpty()) introduction else uploaded.joinToString("\n", "$introduction\n\nAttached files are available at:\n") { "- ${it.name}: ${it.path}" }
                outgoing = outgoing.copy(wireText = message, status = SendStatus.Sending)
                store.updateSend(chat.key, outgoing)
                attempted = true
                client.send(Prompt(outgoing.requestId, sessionId, message), connectionId)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Throwable) {
                outgoing?.let { store.acknowledge(chat.key.connection, it.requestId, false, uncertain = attempted, detail = error.message) }
                if (version == connectionVersion) mutableState.update { it.copy(error = error.message ?: "Message was not sent.") }
            } finally {
                if (version == connectionVersion) mutableState.update { it.copy(uploadingSessions = it.uploadingSessions - sessionId) }
            }
        }
    }

    fun queueControl(sessionId: String, generation: String, operation: QueueOperation) {
        val current = state.value
        val chat = current.transcripts[sessionId] ?: return
        val connectionId = socketId ?: return
        if (current.connectionStatus != ConnectionStatus.Connected) return
        launch {
            val control = store.beginControl(chat.key, generation) { queue ->
                val capability = when (operation) {
                    is QueueOperation.Edit -> "queue_edit"
                    is QueueOperation.Delete -> "queue_delete"
                    is QueueOperation.Prefix -> "queue_run_prefix"
                    is QueueOperation.Pause -> "queue_pause"
                    is QueueOperation.Resume -> "queue_resume"
                    is QueueOperation.Cancel -> "queue_cancel_control"
                }
                check(capability in queue.capabilities) { "Pi does not support this queue operation" }
                val boundary = when (operation) {
                    is QueueOperation.Prefix -> operation.boundary
                    is QueueOperation.Pause -> operation.boundary
                    else -> null
                }
                check(boundary == null || boundary in queue.boundaries) { "Pi does not support this control boundary" }
                operation
            }
            try { client.send(ControlQueue(control.commandId, sessionId, generation, operation), connectionId) }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Throwable) {
                store.acknowledge(chat.key.connection, control.commandId, false, uncertain = true, detail = error.message)
                throw error
            }
        }
    }

    fun restorePending(sessionId: String, id: String) {
        val chat = state.value.transcripts[sessionId] ?: return
        launch {
            store.restoreSend(chat.key, id)
            if (state.value.settings.identity == chat.key.connection) mutableState.update { it.copy(drafts = it.drafts + (sessionId to chat.preferences["draft"].orEmpty())) }
        }
    }

    fun dismissPending(sessionId: String, id: String) {
        val chat = state.value.transcripts[sessionId] ?: return
        launch {
            store.dismissPending(chat.key, id)
        }
    }

    fun dismissExpiredExtensionUi(dialog: SessionExtensionUi) {
        mutableState.update { it.copy(extensionDialogs = it.extensionDialogs.filterNot { active -> active.sessionId == dialog.sessionId && active.request.id == dialog.request.id }) }
    }

    fun respondExtensionUi(dialog: SessionExtensionUi, value: String? = null, confirmed: Boolean? = null, cancelled: Boolean = false) {
        if (state.value.connectionStatus != ConnectionStatus.Connected) return
        dismissExpiredExtensionUi(dialog)
        send(RespondExtensionUi(newRequestId(), dialog.sessionId, dialog.request.id, value, confirmed, cancelled))
    }

    fun downloadAttachment(message: TranscriptEntry) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val attachment = message.attachment ?: return
        val key = AttachmentDownloadKey(sessionId, message.id)
        if (key in downloadJobs) return
        mutableState.update {
            it.copy(
                attachmentDownloads = it.attachmentDownloads + (key to AttachmentDownload(
                    status = AttachmentDownloadStatus.Downloading,
                    transferredBytes = 0,
                    totalBytes = attachment.size,
                )),
                error = null,
            )
        }
        val started = TimeSource.Monotonic.markNow()
        var transferredBytes = 0L
        var totalBytes = attachment.size
        var lastPublishedBytes = 0L
        var lastPublishedMillis = 0L
        lateinit var job: Job
        job = scope.launch(start = CoroutineStart.LAZY) {
            try {
                val download = client.downloadAttachment(
                    mutableState.value.settings,
                    sessionId,
                    message.id,
                    attachment.fileName,
                ) { transferred, total ->
                    transferredBytes = transferred
                    if (total != null) totalBytes = total
                    val elapsedMillis = started.elapsedNow().inWholeMilliseconds.coerceAtLeast(1)
                    if (
                        elapsedMillis - lastPublishedMillis >= DownloadProgressIntervalMillis ||
                        totalBytes?.let { transferred >= it } == true
                    ) {
                        val intervalMillis = (elapsedMillis - lastPublishedMillis).coerceAtLeast(1)
                        val bytesPerSecond = (transferred - lastPublishedBytes).coerceAtLeast(0) *
                            1_000 / intervalMillis
                        lastPublishedBytes = transferred
                        lastPublishedMillis = elapsedMillis
                        mutableState.update { current ->
                            val active = current.attachmentDownloads[key]
                            if (active?.status != AttachmentDownloadStatus.Downloading) {
                                current
                            } else {
                                current.copy(
                                    attachmentDownloads = current.attachmentDownloads +
                                        (key to active.copy(
                                            transferredBytes = transferred,
                                            totalBytes = totalBytes,
                                            bytesPerSecond = bytesPerSecond,
                                        )),
                                )
                            }
                        }
                    }
                }
                mutableState.update { current ->
                    if (downloadJobs[key] !== job) {
                        current
                    } else {
                        current.copy(
                            attachmentDownloads = current.attachmentDownloads +
                                (key to AttachmentDownload(
                                    status = AttachmentDownloadStatus.Downloaded,
                                    transferredBytes = transferredBytes,
                                    totalBytes = totalBytes ?: transferredBytes,
                                    saved = download,
                                )),
                            notice = "Saved to ${download.location}",
                            error = null,
                        )
                    }
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                val rawError = error.message.orEmpty()
                val detail = when {
                    rawError.contains("timeout", ignoreCase = true) -> "Timed out"
                    rawError.contains("HTTP ") -> rawError.substringAfter("Attachment download failed with ")
                    else -> "Download interrupted"
                }
                mutableState.update { current ->
                    if (downloadJobs[key] !== job) {
                        current
                    } else {
                        val active = current.attachmentDownloads[key] ?: AttachmentDownload(
                            status = AttachmentDownloadStatus.Downloading,
                            transferredBytes = transferredBytes,
                            totalBytes = totalBytes,
                        )
                        current.copy(
                            attachmentDownloads = current.attachmentDownloads +
                                (key to active.copy(
                                    status = AttachmentDownloadStatus.Failed,
                                    bytesPerSecond = null,
                                    error = detail,
                                )),
                        )
                    }
                }
            } finally {
                if (downloadJobs[key] === job) downloadJobs.remove(key)
            }
        }
        downloadJobs[key] = job
        job.start()
    }

    fun cancelAttachmentDownload(message: TranscriptEntry) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val key = AttachmentDownloadKey(sessionId, message.id)
        downloadJobs.remove(key)?.cancel()
        mutableState.update {
            it.copy(attachmentDownloads = it.attachmentDownloads - key)
        }
    }

    fun openAttachmentDownload(message: TranscriptEntry) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val key = AttachmentDownloadKey(sessionId, message.id)
        val download = mutableState.value.attachmentDownloads[key]?.saved ?: return
        try {
            PlatformServices.openDownload(download)
        } catch (error: Throwable) {
            mutableState.update {
                it.copy(error = error.message ?: "The downloaded file could not be opened.")
            }
        }
    }

    fun showAttachmentDownload(message: TranscriptEntry) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val key = AttachmentDownloadKey(sessionId, message.id)
        val download = mutableState.value.attachmentDownloads[key]?.saved ?: return
        try {
            PlatformServices.showDownload(download)
        } catch (error: Throwable) {
            mutableState.update {
                it.copy(error = error.message ?: "The downloaded file could not be shown.")
            }
        }
    }

    fun extractAndOpenAttachmentDownload(message: TranscriptEntry) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val key = AttachmentDownloadKey(sessionId, message.id)
        val download = mutableState.value.attachmentDownloads[key]?.saved ?: return
        try {
            PlatformServices.extractAndOpenDownload(download)
        } catch (error: Throwable) {
            mutableState.update {
                it.copy(error = error.message ?: "The downloaded ZIP could not be extracted.")
            }
        }
    }

    fun dispose(): Job {
        closeJob?.let { return it }
        val current = state.value
        val lifetime = checkNotNull(scope.coroutineContext[Job])
        scope.cancel()
        client.close()
        return CoroutineScope(scope.coroutineContext.minusKey(Job)).launch {
            lifetime.join()
            try { store.disconnect(current.settings.identity) }
            finally { store.close() }
        }.also { closeJob = it }
    }

    private fun launch(block: suspend () -> Unit): Job {
        val version = connectionVersion
        return scope.launch(start = CoroutineStart.UNDISPATCHED) {
            try { block() }
            catch (error: Throwable) {
                ensureActive()
                if (version == connectionVersion) mutableState.update { it.copy(error = error.message?.take(240) ?: "Tau operation failed.") }
            }
        }
    }

    private suspend fun loadChat(key: ChatKey): RetainedChat {
        val chat = store.chat(key)
        if (state.value.settings.identity == key.connection) mutableState.update { current ->
            if (key.session in current.transcripts) current else current.copy(
                transcripts = current.transcripts + (key.session to chat),
                drafts = current.drafts + (key.session to chat.preferences["draft"].orEmpty()),
            )
        }
        return chat
    }

    private fun connect(settings: ConnectionSettings) {
        val version = ++connectionVersion
        val prior = connectionJob
        prior?.cancel()
        socketId = null
        pending.clear()
        syncing.clear()
        downloadJobs.values.forEach { it.cancel() }
        downloadJobs.clear()
        val same = state.value.settings.identity == settings.identity
        mutableState.update { previous ->
            val base = if (same) previous else TauUiState()
            base.copy(settings = settings, editingSettings = settings.token.isBlank(), connectionStatus = ConnectionStatus.Connecting,
                restoring = base.transcripts.isEmpty(), error = null, attachmentDownloads = emptyMap(), uploadingSessions = emptySet(), loadingHistory = emptySet())
        }
        connectionJob = scope.launch {
            try {
                prior?.join()
                store.disconnect(settings.identity)
                val retained = store.loadConnection(settings.identity)
                val selected = retained.selected?.takeIf { id -> retained.sessions.any { it.id == id } } ?: retained.sessions.firstOrNull()?.id
                if (selected != null) loadChat(ChatKey(settings.identity, selected))
                mutableState.update { it.copy(sessions = retained.sessions, selectedSessionId = selected, restoring = false,
                    mobileChatVisible = selected != null, connectionStatus = if (settings.token.isBlank()) ConnectionStatus.NotConfigured else ConnectionStatus.Connecting) }
                if (settings.token.isBlank()) return@launch
                var crashUploaded = false
                while (isActive && version == connectionVersion) {
                    mutableState.update { it.copy(connectionStatus = ConnectionStatus.Connecting, daemonVersion = null) }
                    try {
                        client.run(settings) { messages, id ->
                            if (version == connectionVersion) receive(messages, id)
                            if (!crashUploaded && state.value.connectionStatus == ConnectionStatus.Connected) {
                                crashUploaded = true
                                scope.launch { try { client.uploadPendingCrash(settings) } catch (_: Throwable) {} }
                            }
                        }
                    } catch (error: Throwable) {
                        ensureActive()
                        if (version == connectionVersion) mutableState.update { it.copy(error = error.message?.take(240)) }
                    } finally {
                        withContext(NonCancellable) {
                            store.disconnect(settings.identity)
                            if (version == connectionVersion) {
                                socketId = null
                                pending.clear()
                                syncing.clear()
                                mutableState.update { it.copy(connectionStatus = ConnectionStatus.Offline, daemonVersion = null,
                                    slashCommands = emptyMap(), loadingCommands = emptySet(), loadingHistory = emptySet(), extensionDialogs = emptyList(), extensionStatuses = emptyMap(), extensionWidgets = emptyMap()) }
                            }
                        }
                    }
                    delay(ReconnectDelayMillis)
                }
            } catch (error: Throwable) {
                ensureActive()
                if (version == connectionVersion) mutableState.update { it.copy(restoring = false, connectionStatus = ConnectionStatus.Offline, error = "Retained store: ${error.message}") }
            }
        }
    }

    private suspend fun receive(messages: List<ServerMessage>, connectionId: Long) {
        val identity = state.value.settings.identity
        var index = 0
        while (index < messages.size) {
            when (val message = messages[index++]) {
                is Hello -> {
                    check(message.protocolVersion == TauProtocolVersion) { "Tau protocol ${message.protocolVersion} needs a matching client and daemon update" }
                    socketId = connectionId
                    mutableState.update { it.copy(connectionStatus = ConnectionStatus.Connected, daemonVersion = message.daemonVersion, error = null) }
                    state.value.selectedSessionId?.let { openSession(it); loadCommands(it) }
                }
                is TranscriptSnapshot -> {
                    if (message.sessionId != state.value.selectedSessionId) {
                        store.invalidate(identity, message.sessionId)
                        continue
                    }
                    val key = ChatKey(identity, message.sessionId)
                    val chat = loadChat(key)
                    if (store.applySnapshot(key, message.snapshot)) syncing.remove(message.sessionId)
                    else if (!chat.synchronized) {
                        syncing.remove(message.sessionId)
                        openSession(message.sessionId)
                    }
                }
                is TranscriptPage -> {
                    val action = pending[message.requestId] as? PendingAction.History
                    if (action != null && message.sessionId == state.value.selectedSessionId && action.sessionId == message.sessionId &&
                        action.generation == message.generation && action.cursor == message.cursor) {
                        try { store.applyHistory(ChatKey(identity, message.sessionId), message.generation, message.cursor, message.page) }
                        finally {
                            pending.remove(message.requestId)
                            mutableState.update { it.copy(loadingHistory = it.loadingHistory - message.sessionId) }
                        }
                    }
                }
                is TranscriptUpdate -> {
                    if (message.sessionId != state.value.selectedSessionId) {
                        store.invalidate(identity, message.sessionId)
                        continue
                    }
                    val key = ChatKey(identity, message.sessionId)
                    loadChat(key)
                    val patches = mutableListOf(TranscriptPatch(message.generation, message.sequence, message.change))
                    while (index < messages.size) {
                        val next = messages[index] as? TranscriptUpdate ?: break
                        if (next.sessionId != message.sessionId) break
                        patches.add(TranscriptPatch(next.generation, next.sequence, next.change))
                        index++
                    }
                    if (!store.applyUpdates(key, patches)) openSession(message.sessionId)
                }
                is Response -> {
                    store.acknowledge(identity, message.requestId, message.ok, message.uncertain, message.disposition, message.outcome, message.error)
                    val action = pending.remove(message.requestId)
                    if (!message.ok) {
                        if (action is PendingAction.Open) syncing.remove(action.sessionId)
                        if (message.sessionId == null || message.sessionId == state.value.selectedSessionId) {
                            mutableState.update { it.copy(error = (if (message.uncertain) "Unconfirmed: " else "") + (message.error ?: "Tau rejected the request.")) }
                        }
                    } else {
                        if (action is PendingAction.Delete) {
                            store.removeChat(ChatKey(identity, action.sessionId))
                            mutableState.update { it.copy(transcripts = it.transcripts - action.sessionId, drafts = it.drafts - action.sessionId) }
                        }
                        if ((action == PendingAction.Create || action == PendingAction.Select) && message.sessionId != null) {
                            val key = ChatKey(identity, message.sessionId)
                            loadChat(key)
                            if (message.draft != null) {
                                store.setPreference(key, "draft", message.draft)
                                mutableState.update { it.copy(drafts = it.drafts + (key.session to message.draft)) }
                            }
                            mutableState.update { it.copy(focusComposerSessionId = key.session) }
                            selectSession(key.session)
                        }
                        if (message.notice != null) mutableState.update { it.copy(notice = message.notice) }
                        if (message.outcome != null && message.outcome != "accepted") mutableState.update { it.copy(notice = "Queue: ${message.outcome.replace('_', ' ')}") }
                    }
                    if (action is PendingAction.Commands) mutableState.update { it.copy(loadingCommands = it.loadingCommands - action.sessionId) }
                    if (action is PendingAction.History) mutableState.update { it.copy(loadingHistory = it.loadingHistory - action.sessionId) }
                }
                is Sessions -> {
                    store.saveSessions(identity, message.sessions)
                    val ids = message.sessions.mapTo(mutableSetOf()) { it.id }
                    val selected = state.value.selectedSessionId?.takeIf { it in ids } ?: message.sessions.firstOrNull()?.id
                    mutableState.update { it.copy(sessions = message.sessions, selectedSessionId = selected, mobileChatVisible = it.mobileChatVisible && selected != null) }
                    if (selected != null) {
                        val key = ChatKey(identity, selected)
                        val chat = loadChat(key)
                        store.select(key)
                        if (!chat.synchronized) openSession(selected)
                    }
                }
                is Commands -> mutableState.update { it.copy(slashCommands = it.slashCommands + (message.sessionId to message.commands), loadingCommands = it.loadingCommands - message.sessionId) }
                is SessionState -> {
                    val stopped = message.status == SessionStatus.Sleeping || message.status == SessionStatus.Error
                    val sessions = state.value.sessions.map { if (it.id == message.sessionId) it.copy(status = message.status, detail = message.detail, contextUsage = message.contextUsage) else it }
                    store.updateSessionState(identity, message)
                    mutableState.update { it.copy(sessions = sessions,
                        slashCommands = if (stopped) it.slashCommands - message.sessionId else it.slashCommands,
                        loadingCommands = if (stopped) it.loadingCommands - message.sessionId else it.loadingCommands,
                        extensionDialogs = if (stopped) it.extensionDialogs.filterNot { dialog -> dialog.sessionId == message.sessionId } else it.extensionDialogs,
                        extensionStatuses = if (stopped) it.extensionStatuses - message.sessionId else it.extensionStatuses,
                        extensionWidgets = if (stopped) it.extensionWidgets - message.sessionId else it.extensionWidgets) }
                }
                is ExtensionError -> mutableState.update { it.copy(error = message.error) }
                is ExtensionUi -> {
                    if (message.request.method == "set_editor_text") {
                        setDraft(message.sessionId, message.request.text.orEmpty())
                    } else {
                        mutableState.update { current ->
                            val request = message.request
                            val sessionId = message.sessionId
                            when (request.method) {
                                "select", "confirm", "input", "editor" -> if (current.extensionDialogs.any { it.sessionId == sessionId && it.request.id == request.id }) current
                                    else current.copy(extensionDialogs = current.extensionDialogs + SessionExtensionUi(sessionId, request))
                                "notify" -> if (request.notifyType == "error") current.copy(error = request.message ?: "Pi extension failed.") else current.copy(notice = request.message)
                                "setStatus" -> {
                                    val statuses = current.extensionStatuses[sessionId].orEmpty()
                                    val updated = if (request.statusKey == null) statuses else if (request.statusText == null) statuses - request.statusKey else statuses + (request.statusKey to request.statusText)
                                    current.copy(extensionStatuses = current.extensionStatuses + (sessionId to updated))
                                }
                                "setWidget" -> {
                                    val widgets = current.extensionWidgets[sessionId].orEmpty()
                                    val updated = if (request.widgetKey == null) widgets else if (request.widgetLines.isEmpty()) widgets - request.widgetKey
                                        else widgets + (request.widgetKey to ExtensionWidget(request.widgetLines, request.widgetPlacement))
                                    current.copy(extensionWidgets = current.extensionWidgets + (sessionId to updated))
                                }
                                "setTitle" -> current
                                else -> current.copy(error = "Pi requested unsupported extension UI: ${request.method}")
                            }
                        }
                    }
                }
                is ResyncRequired -> {
                    store.invalidate(identity, message.sessionId)
                    if (message.sessionId == null) send(ListSessions(newRequestId()))
                    val selected = state.value.selectedSessionId
                    if (selected != null && (message.sessionId == null || message.sessionId == selected)) openSession(selected)
                }
            }
        }
    }

    private fun openSession(sessionId: String) {
        if (socketId == null || state.value.selectedSessionId != sessionId || !syncing.add(sessionId)) return
        pending.entries.removeAll { (_, action) -> action is PendingAction.History && action.sessionId == sessionId }
        mutableState.update { it.copy(loadingHistory = it.loadingHistory - sessionId) }
        val chat = state.value.transcripts[sessionId]
        send(OpenSession(newRequestId(), sessionId, chat?.pending?.map { it.requestId }.orEmpty(),
            chat?.byId?.values?.filter { it.entry.phase != EntryPhase.Saved }?.mapNotNull { it.entry.origin.streamId }.orEmpty()), PendingAction.Open(sessionId))
    }

    private fun send(request: ClientRequest, action: PendingAction = PendingAction.Normal) {
        val connectionId = socketId ?: return
        val version = connectionVersion
        pending[request.id] = action
        launch {
            try {
                client.send(request, connectionId)
                if (action is PendingAction.History) {
                    delay(CommandLoadMillis)
                    if (pending.remove(request.id) == action) mutableState.update {
                        it.copy(loadingHistory = it.loadingHistory - action.sessionId, error = "History page timed out")
                    }
                }
                if (action is PendingAction.Commands) {
                    delay(CommandLoadMillis)
                    if (pending.remove(request.id) == action) mutableState.update {
                        it.copy(loadingCommands = it.loadingCommands - action.sessionId, error = "Pi command list timed out")
                    }
                }
            }
            catch (error: Throwable) {
                if (version == connectionVersion) {
                    pending.remove(request.id)
                    if (action is PendingAction.Open) syncing.remove(action.sessionId)
                    if (action is PendingAction.Commands) mutableState.update { it.copy(loadingCommands = it.loadingCommands - action.sessionId) }
                    if (action is PendingAction.History) mutableState.update { it.copy(loadingHistory = it.loadingHistory - action.sessionId) }
                }
                throw error
            }
        }
    }

    private sealed interface PendingAction {
        data object Normal : PendingAction
        data object Create : PendingAction
        data object Select : PendingAction
        data class Delete(val sessionId: String) : PendingAction
        data class Open(val sessionId: String) : PendingAction
        data class Commands(val sessionId: String) : PendingAction
        data class History(val sessionId: String, val generation: String, val cursor: String) : PendingAction
    }
}
