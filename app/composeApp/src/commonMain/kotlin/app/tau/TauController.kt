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
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.contentOrNull
import kotlin.time.Duration.Companion.minutes
import kotlin.time.Duration.Companion.seconds
import kotlin.time.TimeSource
import okio.ByteString.Companion.encodeUtf8
import okio.FileSystem
import okio.Path.Companion.toPath

private const val ReconnectDelayMillis = 2_000L
private const val CommandLoadMillis = 10_000L
private const val BackgroundReads = 2
private const val RecentWarmChats = 5
private const val WarmHistoryEvents = HistoryPageEvents * 3
private const val WarmHistoryBytes = HistoryPageBytes * 3
internal const val DownloadProgressIntervalMillis = 200L

enum class ConnectionStatus { NotConfigured, Connecting, Connected, Offline }

data class SessionExtensionUi(val sessionId: String, val request: ExtensionUiRequest)

internal const val UsageNoticePrefix = "VIBE_BRIDGE_CODEX_USAGE:"
private val UsageRequestTimeout = 30.seconds
private val UsageStaleAfter = 5.minutes

data class CodexUsageWindow(
    val id: String,
    val label: String,
    val durationSeconds: Long? = null,
    val remainingPercent: Double? = null,
    val resetsAtMs: Long? = null,
)

data class CodexUsage(
    val provider: String,
    val fetchedAtMs: Long,
    val plan: String? = null,
    val limitReached: Boolean = false,
    val windows: List<CodexUsageWindow> = emptyList(),
) {
    internal val received = TimeSource.Monotonic.markNow()
}

internal data class UsageNoticeResult(val usage: CodexUsage?, val failure: String?)

internal fun parseUsageNotice(notice: String, expectedRequestId: Long): UsageNoticeResult {
    val root = try { Json.parseToJsonElement(notice.removePrefix(UsageNoticePrefix)).jsonObject }
        catch (_: Exception) { return UsageNoticeResult(null, null) }
    if (root["version"]?.jsonPrimitive?.longOrNull != 2L) return UsageNoticeResult(null, null)
    if (root["requestId"]?.jsonPrimitive?.longOrNull != expectedRequestId) return UsageNoticeResult(null, null)
    val text = root["text"]?.jsonPrimitive?.contentOrNull
    val report = root["report"]?.jsonObject ?: return UsageNoticeResult(null, text)
    val windows = report["windows"]?.jsonArray?.mapNotNull { window ->
        val entry = window.jsonObject
        val id = entry["id"]?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null
        CodexUsageWindow(
            id = id,
            label = entry["label"]?.jsonPrimitive?.contentOrNull ?: id,
            durationSeconds = entry["durationSeconds"]?.jsonPrimitive?.longOrNull,
            remainingPercent = entry["remainingPercent"]?.jsonPrimitive?.doubleOrNull,
            resetsAtMs = entry["resetsAtMs"]?.jsonPrimitive?.longOrNull,
        )
    }.orEmpty()
    val fetchedAtMs = report["fetchedAtMs"]?.jsonPrimitive?.longOrNull ?: return UsageNoticeResult(null, text)
    val usage = CodexUsage(
        provider = report["provider"]?.jsonPrimitive?.contentOrNull ?: "openai-codex",
        fetchedAtMs = fetchedAtMs,
        plan = report["plan"]?.jsonPrimitive?.contentOrNull,
        limitReached = report["limitReached"]?.jsonPrimitive?.booleanOrNull ?: false,
        windows = windows,
    )
    return UsageNoticeResult(usage, null)
}
data class ExtensionWidget(val lines: List<String>, val placement: String?)
data class AttachmentDownloadKey(val sessionId: String, val entryId: String)

enum class AttachmentDownloadAction { Preview, Reload, Save }
enum class AttachmentDownloadStatus { Downloading, Downloaded, Failed }

data class AttachmentDownload(
    val status: AttachmentDownloadStatus,
    val transferredBytes: Long,
    val totalBytes: Long?,
    val bytesPerSecond: Long? = null,
    val saved: SavedDownload? = null,
    val failure: AttachmentFailure? = null,
    val localPath: String? = null,
    val attempt: Int = 0,
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
    val codexUsage: CodexUsage? = null,
    val seenHeads: Map<String, String> = emptyMap(),
    val unread: Set<String> = emptySet(),
    val attachmentDownloads: Map<AttachmentDownloadKey, AttachmentDownload> = emptyMap(),
    val pickingFiles: Boolean = false,
    val uploadingSessions: Set<String> = emptySet(),
    val mobileChatVisible: Boolean = false,
    val notice: String? = null,
    val error: String? = null,
)

class TauController(
    internal val dispatcher: CoroutineDispatcher,
    private val store: LocalStore = LocalStore({ PlatformServices.transcriptDatabasePath }),
) {
    internal val scope = CoroutineScope(SupervisorJob() + dispatcher)
    internal val client = TauClient()
    internal val mutableState = MutableStateFlow(TauUiState())
    private val pending = mutableMapOf<String, PendingAction>()
    private val failedReads = mutableSetOf<String>()
    internal val downloadJobs = mutableMapOf<AttachmentDownloadKey, Job>()
    private var usageRequest: Pair<Long, TimeSource.Monotonic.ValueTimeMark>? = null
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
        mutableState.update { it.copy(unread = it.unread - sessionId) }
        mutableState.update { it.copy(selectedSessionId = sessionId, mobileChatVisible = true, error = null) }
        failedReads.remove(sessionId)
        val key = ChatKey(state.value.settings.identity, sessionId)
        launch {
            val chat = loadChat(key)
            if (state.value.settings.identity == key.connection && state.value.selectedSessionId == sessionId) {
                store.select(key)
                if (!chat.synchronized) openSession(sessionId)
                loadCommands(sessionId)
                warmChats()
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
        val cursor = chat.before
        if (cursor == null) {
            mutableState.update { it.copy(loadingHistory = it.loadingHistory - sessionId) }
            return
        }
        if (pending.values.any { it is PendingAction.History && it.sessionId == sessionId }) return
        failedReads.remove(sessionId)
        mutableState.update { it.copy(loadingHistory = it.loadingHistory + sessionId, error = if (sessionId == it.selectedSessionId) null else it.error) }
        if (socketId == null) return
        if (!chat.synchronized) { openSession(sessionId); return }
        val request = GetHistory(newRequestId(), sessionId, chat.position.generation, cursor)
        send(request, PendingAction.History(sessionId, request.generation, cursor))
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


    fun refreshUsage(sessionId: String, force: Boolean = false) {
        val current = mutableState.value
        if (current.selectedSessionId != sessionId || current.connectionStatus != ConnectionStatus.Connected) return
        val inFlight = usageRequest
        if (inFlight != null && inFlight.second.elapsedNow() < UsageRequestTimeout) return
        val usage = current.codexUsage
        if (!force && usage != null && usage.received.elapsedNow() < UsageStaleAfter) return
        val requestId = PlatformServices.epochMillis()
        usageRequest = requestId to TimeSource.Monotonic.markNow()
        send(Prompt(newRequestId(), sessionId, "/vibe-bridge-usage $requestId"))
    }

    private fun onUsageNotice(notice: String) {
        val inFlight = usageRequest ?: return
        if (inFlight.second.elapsedNow() > UsageRequestTimeout) { usageRequest = null; return }
        val result = parseUsageNotice(notice, inFlight.first)
        usageRequest = null
        when {
            result == null -> return
            result.usage != null -> mutableState.update { it.copy(codexUsage = result.usage) }
            result.failure != null -> mutableState.update { it.copy(notice = result.failure) }
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
        val chat = state.value.transcripts[key.session]?.takeIf { state.value.settings.identity == key.connection } ?: store.chat(key)
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
        failedReads.clear()
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
                                failedReads.clear()
                                mutableState.update { it.copy(connectionStatus = ConnectionStatus.Offline, daemonVersion = null,
                                    slashCommands = emptyMap(), loadingCommands = emptySet(), extensionDialogs = emptyList(), extensionStatuses = emptyMap(), extensionWidgets = emptyMap()) }
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
                    val key = ChatKey(identity, message.sessionId)
                    val chat = loadChat(key)
                    if (store.applySnapshot(key, message.snapshot)) {
                        pending.entries.removeAll { (_, action) -> action is PendingAction.Open && action.sessionId == message.sessionId ||
                            action is PendingAction.History && action.sessionId == message.sessionId && (action.generation != chat.position.generation || action.cursor != chat.before) }
                        failedReads.remove(message.sessionId)
                    } else if (!chat.synchronized) {
                        pending.entries.removeAll { (_, action) -> action is PendingAction.Open && action.sessionId == message.sessionId }
                        openSession(message.sessionId)
                    }
                }
                is TranscriptPage -> {
                    val action = pending[message.requestId] as? PendingAction.History
                    if (action != null && action.sessionId == message.sessionId &&
                        action.generation == message.generation && action.cursor == message.cursor) {
                        try { store.applyHistory(ChatKey(identity, message.sessionId), message.generation, message.cursor, message.page) }
                        finally {
                            pending.remove(message.requestId)
                            mutableState.update { it.copy(loadingHistory = it.loadingHistory - message.sessionId) }
                        }
                    }
                }
                is TranscriptUpdate -> {
                    val key = ChatKey(identity, message.sessionId)
                    loadChat(key)
                    val patches = mutableListOf(TranscriptPatch(message.generation, message.sequence, message.change))
                    while (index < messages.size) {
                        val next = messages[index] as? TranscriptUpdate ?: break
                        if (next.sessionId != message.sessionId) break
                        patches.add(TranscriptPatch(next.generation, next.sequence, next.change))
                        index++
                    }
                    if (!store.applyUpdates(key, patches) && message.sessionId == state.value.selectedSessionId) openSession(message.sessionId)
                }
                is Response -> {
                    val action = pending.remove(message.requestId)
                    if (action == null && state.value.transcripts.values.none { chat ->
                        chat.pending.any { it.requestId == message.requestId } || chat.controls.any { it.commandId == message.requestId }
                    }) continue
                    store.acknowledge(identity, message.requestId, message.ok, message.uncertain, message.disposition, message.outcome, message.error)
                    if (!message.ok) {
                        action?.readSession?.let { sessionId ->
                            failedReads.add(sessionId)
                            store.invalidate(identity, sessionId)
                            mutableState.update { it.copy(loadingHistory = it.loadingHistory - sessionId) }
                        }
                        if ((action?.readSession == null || action.readSession == state.value.selectedSessionId) &&
                            (message.sessionId == null || message.sessionId == state.value.selectedSessionId)) {
                            mutableState.update { it.copy(error = (if (message.uncertain) "Unconfirmed: " else "") + (message.error ?: "Tau rejected the request.")) }
                        }
                    } else {
                        if (action is PendingAction.Delete) {
                            for ((key, job) in downloadJobs.toMap()) if (key.sessionId == action.sessionId) {
                                downloadJobs.remove(key); job.cancel(); job.join()
                            }
                            store.removeChat(ChatKey(identity, action.sessionId))
                            withContext(Dispatchers.IO) {
                                FileSystem.SYSTEM.deleteRecursively(PlatformServices.attachmentDirectory.toPath() / identity /
                                    action.sessionId.encodeUtf8().sha256().hex(), mustExist = false)
                            }
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
                    mutableState.update { current ->
                        var seenHeads = current.seenHeads
                        val unread = current.unread.toMutableSet()
                        unread.retainAll(ids)
                        for (session in message.sessions) {
                            val head = session.lastEntryId ?: continue
                            val known = current.seenHeads[session.id]
                            when {
                                known == null -> seenHeads = seenHeads + (session.id to head)
                                known != head -> {
                                    seenHeads = seenHeads + (session.id to head)
                                    if (session.id != selected) unread += session.id
                                }
                            }
                        }
                        current.copy(sessions = message.sessions, selectedSessionId = selected,
                            mobileChatVisible = current.mobileChatVisible && selected != null,
                            seenHeads = seenHeads, unread = unread)
                    }
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
                    } else if (message.request.method == "notify" && message.request.message?.startsWith(UsageNoticePrefix) == true) {
                        onUsageNotice(message.request.message)
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
                    if (message.sessionId == null) { failedReads.clear(); send(ListSessions(newRequestId())) }
                    else failedReads.remove(message.sessionId)
                    val selected = state.value.selectedSessionId
                    if (selected != null && (message.sessionId == null || message.sessionId == selected)) openSession(selected)
                }
            }
        }
        warmChats()
    }

    private suspend fun warmChats() {
        val current = state.value
        if (socketId == null || current.connectionStatus != ConnectionStatus.Connected) return
        val candidates = (listOfNotNull(current.selectedSessionId) +
            current.sessions.filter { it.status == SessionStatus.Running || it.status == SessionStatus.Starting }.map { it.id } +
            current.sessions.filter { it.id in current.unread }.map { it.id } +
            current.sessions.take(RecentWarmChats).map { it.id } +
            current.sessions.filter { it.id in current.transcripts }.map { it.id }).distinct()
        for (sessionId in candidates) {
            if (sessionId in failedReads || pending.values.any { it.readSession == sessionId }) continue
            if (sessionId != state.value.selectedSessionId && pending.values.mapNotNull { it.readSession }.distinct().count { it != state.value.selectedSessionId } >= BackgroundReads) break
            val chat = loadChat(ChatKey(current.settings.identity, sessionId))
            if (state.value.settings.identity != current.settings.identity || socketId == null) return
            val selected = state.value.selectedSessionId
            if (sessionId != selected && pending.values.mapNotNull { it.readSession }.distinct().count { it != selected } >= BackgroundReads) continue
            if (!chat.synchronized) openSession(sessionId)
            else if (sessionId in state.value.loadingHistory || chat.before != null && chat.rows.size < WarmHistoryEvents && chat.rows.sumOf { it.event.pageBytes } < WarmHistoryBytes) loadOlder(sessionId)
        }
    }

    private fun openSession(sessionId: String) {
        if (socketId == null || pending.values.any { it is PendingAction.Open && it.sessionId == sessionId }) return
        pending.entries.removeAll { (_, action) -> action is PendingAction.History && action.sessionId == sessionId }
        val chat = state.value.transcripts[sessionId]
        send(OpenSession(newRequestId(), sessionId, chat?.pending?.map { it.requestId }.orEmpty()), PendingAction.Open(sessionId))
    }

    private fun send(request: ClientRequest, action: PendingAction = PendingAction.Normal) {
        val connectionId = socketId ?: return
        val version = connectionVersion
        pending[request.id] = action
        launch {
            try {
                client.send(request, connectionId)
                val readSession = action.readSession
                if (readSession != null) {
                    delay(CommandLoadMillis)
                    if (pending.remove(request.id) == action) {
                        failedReads.add(readSession)
                        mutableState.update { it.copy(loadingHistory = it.loadingHistory - readSession,
                            error = if (readSession == it.selectedSessionId) "History read timed out" else it.error) }
                        warmChats()
                    }
                }
                if (action is PendingAction.Commands) {
                    delay(CommandLoadMillis)
                    if (pending.remove(request.id) == action) mutableState.update {
                        it.copy(loadingCommands = it.loadingCommands - action.sessionId, error = "Pi command list timed out")
                    }
                }
            }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Throwable) {
                if (version == connectionVersion) {
                    pending.remove(request.id)
                    if (action is PendingAction.Commands) mutableState.update { it.copy(loadingCommands = it.loadingCommands - action.sessionId) }
                    if (action.readSession != null && error !is TauConnectionException) {
                        failedReads.add(action.readSession!!)
                        mutableState.update { it.copy(loadingHistory = it.loadingHistory - action.readSession!!) }
                    }
                }
                if (action.readSession == null || action.readSession == state.value.selectedSessionId) throw error
                if (error !is TauConnectionException) warmChats()
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
        data class History(val sessionId: String, val generation: String, val cursor: Long) : PendingAction
        val readSession: String? get() = when (this) { is Open -> sessionId; is History -> sessionId; else -> null }
    }
}
