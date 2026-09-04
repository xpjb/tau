package app.tau

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlin.time.TimeSource

private const val ReconnectDelayMillis = 2_000L
private const val DownloadProgressIntervalMillis = 200L

enum class ConnectionStatus {
    NotConfigured,
    Connecting,
    Connected,
    Offline,
}

data class OutgoingMessage(
    val requestId: String,
    val text: String,
    val afterEntryId: String?,
    val occurrence: Int,
)

data class SessionExtensionUi(
    val sessionId: String,
    val request: ExtensionUiRequest,
)

data class ExtensionWidget(
    val lines: List<String>,
    val placement: String?,
)

data class AttachmentDownloadKey(val sessionId: String, val entryId: String)

enum class AttachmentDownloadStatus {
    Downloading,
    Downloaded,
    Failed,
}

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
    val connectionStatus: ConnectionStatus = ConnectionStatus.NotConfigured,
    val daemonVersion: String? = null,
    val sessions: List<SessionSummary> = emptyList(),
    val selectedSessionId: String? = null,
    val focusComposerSessionId: String? = null,
    val histories: Map<String, List<ChatMessage>> = emptyMap(),
    val outgoingMessages: Map<String, List<OutgoingMessage>> = emptyMap(),
    val partials: Map<String, String> = emptyMap(),
    val drafts: Map<String, String> = emptyMap(),
    val attachments: Map<String, List<PickedFile>> = emptyMap(),
    val slashCommands: Map<String, List<SlashCommand>> = emptyMap(),
    val loadingCommands: Set<String> = emptySet(),
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

class TauController(dispatcher: CoroutineDispatcher) {
    private val scope = CoroutineScope(SupervisorJob() + dispatcher)
    private val client = TauClient()
    private val mutableState = MutableStateFlow(TauUiState())
    private val pending = mutableMapOf<String, PendingAction>()
    private val openedSessions = mutableSetOf<String>()
    private val downloadJobs = mutableMapOf<AttachmentDownloadKey, Job>()
    private var connectionJob: Job? = null
    private var requestSequence = 1L
    private var crashUploadAttempted = false
    private var reportNextConnectionError = false

    val state: StateFlow<TauUiState> = mutableState.asStateFlow()

    fun start() {
        val settings = PlatformServices.loadConnection()
        mutableState.update {
            it.copy(
                settings = settings,
                editingSettings = settings.token.isBlank(),
                connectionStatus = if (settings.token.isBlank()) {
                    ConnectionStatus.NotConfigured
                } else {
                    ConnectionStatus.Connecting
                },
            )
        }
        if (settings.token.isNotBlank()) connect(settings)
    }

    fun saveConnection(serverUrl: String, token: String) {
        val normalized = ConnectionSettings(serverUrl.trim().trimEnd('/'), token.trim())
        if ((!normalized.serverUrl.startsWith("http://") &&
                !normalized.serverUrl.startsWith("https://")) || normalized.token.isBlank()
        ) {
            mutableState.update { it.copy(error = "Enter an HTTP server URL and token.") }
            return
        }
        PlatformServices.saveConnection(normalized)
        reportNextConnectionError = true
        mutableState.update {
            it.copy(
                settings = normalized,
                editingSettings = false,
                connectionStatus = ConnectionStatus.Connecting,
                error = null,
            )
        }
        connect(normalized)
    }

    fun showSettings() {
        mutableState.update { it.copy(editingSettings = true) }
    }

    fun hideSettings() {
        if (mutableState.value.settings.token.isNotBlank()) {
            mutableState.update { it.copy(editingSettings = false) }
        }
    }

    fun createSession() {
        val id = nextRequestId()
        pending[id] = PendingAction.CreateSession
        send(CreateSession(id))
    }

    fun selectSession(sessionId: String) {
        mutableState.update {
            it.copy(selectedSessionId = sessionId, mobileChatVisible = true, error = null)
        }
        openSession(sessionId, true)
    }

    fun showSessionList() {
        mutableState.update { it.copy(mobileChatVisible = false) }
    }

    fun consumeComposerFocus(sessionId: String) {
        mutableState.update {
            if (it.focusComposerSessionId == sessionId) {
                it.copy(focusComposerSessionId = null)
            } else {
                it
            }
        }
    }

    fun setDraft(sessionId: String, draft: String) {
        val current = mutableState.value
        val loadCommands = draft.startsWith('/') &&
            current.connectionStatus == ConnectionStatus.Connected &&
            sessionId !in current.slashCommands &&
            sessionId !in current.loadingCommands
        mutableState.update {
            it.copy(
                drafts = it.drafts + (sessionId to draft),
                loadingCommands = if (loadCommands) {
                    it.loadingCommands + sessionId
                } else {
                    it.loadingCommands
                },
            )
        }
        if (loadCommands) {
            val id = nextRequestId()
            pending[id] = PendingAction.Commands(sessionId)
            send(GetCommands(id, sessionId))
        }
    }

    fun pickFiles() {
        val sessionId = mutableState.value.selectedSessionId ?: return
        loadAttachments(sessionId, PlatformServices::pickFiles)
    }

    fun attachDroppedFiles(fileUris: List<String>) {
        if (fileUris.isEmpty()) return
        val sessionId = mutableState.value.selectedSessionId ?: return
        loadAttachments(sessionId) { PlatformServices.readDroppedFiles(fileUris) }
    }

    fun attachClipboardImage(load: suspend () -> PickedFile) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        loadAttachments(sessionId) { listOf(load()) }
    }

    private fun loadAttachments(sessionId: String, load: suspend () -> List<PickedFile>) {
        val current = mutableState.value
        if (current.connectionStatus != ConnectionStatus.Connected ||
            current.pickingFiles || sessionId in current.uploadingSessions
        ) {
            return
        }
        mutableState.update { it.copy(pickingFiles = true, error = null) }
        scope.launch {
            try {
                val selected = load()
                if (selected.isEmpty()) return@launch
                mutableState.update { state ->
                    val files = state.attachments[sessionId].orEmpty() + selected
                    check(files.size <= MaxUploadFiles) {
                        "Attach at most $MaxUploadFiles files to one message"
                    }
                    check(files.sumOf { it.bytes.size.toLong() } <= MaxUploadBytes) {
                        "Attached files exceed Tau's $MaxUploadBytes byte limit"
                    }
                    state.copy(
                        attachments = state.attachments + (sessionId to files),
                        error = null,
                    )
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                mutableState.update {
                    it.copy(error = error.message ?: "Files could not be attached.")
                }
            } finally {
                mutableState.update { it.copy(pickingFiles = false) }
            }
        }
    }

    fun removeAttachment(sessionId: String, index: Int) {
        mutableState.update { current ->
            val files = current.attachments[sessionId].orEmpty()
                .filterIndexed { fileIndex, _ -> fileIndex != index }
            current.copy(
                attachments = if (files.isEmpty()) {
                    current.attachments - sessionId
                } else {
                    current.attachments + (sessionId to files)
                },
            )
        }
    }

    fun sendPrompt() {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val text = mutableState.value.drafts[sessionId].orEmpty()
        val files = mutableState.value.attachments[sessionId].orEmpty()
        if ((text.isBlank() && files.isEmpty()) || sessionId in mutableState.value.uploadingSessions) {
            return
        }
        mutableState.update {
            it.copy(
                uploadingSessions = it.uploadingSessions + sessionId,
                error = null,
            )
        }
        scope.launch {
            var requestId: String? = null
            try {
                val uploaded = files.map { file ->
                    client.uploadFile(mutableState.value.settings, sessionId, file)
                }
                val introduction = if (text.isBlank()) {
                    if (uploaded.size == 1) {
                        "Please inspect the attached file."
                    } else {
                        "Please inspect the attached files."
                    }
                } else {
                    text
                }
                val message = if (uploaded.isEmpty()) {
                    introduction
                } else {
                    uploaded.joinToString(
                        separator = "\n",
                        prefix = "$introduction\n\nAttached files are available at:\n",
                    ) { file -> "- ${file.name}: ${file.path}" }
                }
                val id = nextRequestId()
                requestId = id
                val afterEntryId = mutableState.value.histories[sessionId]
                    .orEmpty()
                    .lastOrNull()
                    ?.entryId
                val occurrence = mutableState.value.outgoingMessages[sessionId]
                    .orEmpty()
                    .count { outgoing -> outgoing.afterEntryId == afterEntryId } +
                    pending.values.count { action ->
                        action is PendingAction.Prompt &&
                            action.sessionId == sessionId &&
                            action.afterEntryId == afterEntryId
                    }
                pending[id] = PendingAction.Prompt(
                    sessionId = sessionId,
                    text = text,
                    files = files,
                    displayText = introduction,
                    afterEntryId = afterEntryId,
                    occurrence = occurrence,
                )
                client.send(Prompt(id, sessionId, message))
                mutableState.update { current ->
                    val remaining = current.attachments[sessionId].orEmpty()
                        .filterNot { candidate -> files.any { it === candidate } }
                    current.copy(
                        drafts = if (current.drafts[sessionId] == text) {
                            current.drafts + (sessionId to "")
                        } else {
                            current.drafts
                        },
                        attachments = if (remaining.isEmpty()) {
                            current.attachments - sessionId
                        } else {
                            current.attachments + (sessionId to remaining)
                        },
                        error = null,
                    )
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                requestId?.let(pending::remove)
                mutableState.update {
                    if (error is TauConnectionException) {
                        it.copy(connectionStatus = ConnectionStatus.Offline)
                    } else {
                        it.copy(error = error.message ?: "Message was not sent.")
                    }
                }
            } finally {
                mutableState.update {
                    it.copy(uploadingSessions = it.uploadingSessions - sessionId)
                }
            }
        }
    }

    fun dismissExpiredExtensionUi(dialog: SessionExtensionUi) {
        mutableState.update {
            it.copy(extensionDialogs = it.extensionDialogs.filterNot { active ->
                active.sessionId == dialog.sessionId && active.request.id == dialog.request.id
            })
        }
    }

    fun respondExtensionUi(
        dialog: SessionExtensionUi,
        value: String? = null,
        confirmed: Boolean? = null,
        cancelled: Boolean = false,
    ) {
        if (mutableState.value.connectionStatus != ConnectionStatus.Connected) return
        mutableState.update {
            it.copy(extensionDialogs = it.extensionDialogs.filterNot { active ->
                active.sessionId == dialog.sessionId && active.request.id == dialog.request.id
            })
        }
        val id = nextRequestId()
        pending[id] = PendingAction.ExtensionUi(dialog)
        send(
            RespondExtensionUi(
                id = id,
                sessionId = dialog.sessionId,
                requestId = dialog.request.id,
                value = value,
                confirmed = confirmed,
                cancelled = cancelled,
            ),
        )
    }

    fun abort() {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val id = nextRequestId()
        pending[id] = PendingAction.Normal
        send(Abort(id, sessionId))
    }

    fun deleteSession(sessionId: String) {
        val id = nextRequestId()
        pending[id] = PendingAction.Normal
        send(DeleteSession(id, sessionId))
    }

    fun renameSession(sessionId: String, title: String) {
        val id = nextRequestId()
        pending[id] = PendingAction.Normal
        send(RenameSession(id, sessionId, title))
    }

    fun fork(entryId: String) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val id = nextRequestId()
        pending[id] = PendingAction.SelectSession
        send(ForkSession(id, sessionId, entryId))
    }

    fun downloadAttachment(message: ChatMessage) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val attachment = message.attachment ?: return
        val key = AttachmentDownloadKey(sessionId, message.entryId)
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
                    message.entryId,
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

    fun cancelAttachmentDownload(message: ChatMessage) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val key = AttachmentDownloadKey(sessionId, message.entryId)
        downloadJobs.remove(key)?.cancel()
        mutableState.update {
            it.copy(attachmentDownloads = it.attachmentDownloads - key)
        }
    }

    fun openAttachmentDownload(message: ChatMessage) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val key = AttachmentDownloadKey(sessionId, message.entryId)
        val download = mutableState.value.attachmentDownloads[key]?.saved ?: return
        try {
            PlatformServices.openDownload(download)
        } catch (error: Throwable) {
            mutableState.update {
                it.copy(
                    attachmentDownloads = it.attachmentDownloads - key,
                    error = error.message ?: "The downloaded file could not be opened.",
                )
            }
        }
    }

    fun dismissError() {
        mutableState.update { it.copy(error = null) }
    }

    fun dismissNotice() {
        mutableState.update { it.copy(notice = null) }
    }

    fun dispose() {
        connectionJob?.cancel()
        client.close()
        scope.cancel()
    }

    private fun connect(settings: ConnectionSettings) {
        connectionJob?.cancel()
        downloadJobs.values.forEach { job -> job.cancel() }
        downloadJobs.clear()
        openedSessions.clear()
        pending.clear()
        mutableState.update { it.copy(attachmentDownloads = emptyMap()) }
        crashUploadAttempted = false
        connectionJob = scope.launch {
            while (isActive) {
                mutableState.update {
                    it.copy(connectionStatus = ConnectionStatus.Connecting, daemonVersion = null)
                }
                try {
                    client.run(settings, ::receive)
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (error: Throwable) {
                    val connectionError = if (reportNextConnectionError) {
                        error.message?.take(240) ?: "Tau connection failed."
                    } else {
                        null
                    }
                    reportNextConnectionError = false
                    mutableState.update {
                        it.copy(
                            connectionStatus = ConnectionStatus.Offline,
                            error = connectionError ?: it.error,
                        )
                    }
                }
                openedSessions.clear()
                pending.clear()
                mutableState.update {
                    it.copy(
                        connectionStatus = ConnectionStatus.Offline,
                        daemonVersion = null,
                        slashCommands = emptyMap(),
                        loadingCommands = emptySet(),
                        extensionDialogs = emptyList(),
                        extensionStatuses = emptyMap(),
                        extensionWidgets = emptyMap(),
                    )
                }
                delay(ReconnectDelayMillis)
            }
        }
    }

    private suspend fun receive(message: ServerMessage) {
        when (message) {
            is Hello -> {
                check(message.protocolVersion == TauProtocolVersion) {
                    "Tau protocol ${message.protocolVersion} is not supported"
                }
                reportNextConnectionError = false
                mutableState.update {
                    it.copy(
                        connectionStatus = ConnectionStatus.Connected,
                        daemonVersion = message.daemonVersion,
                        error = null,
                    )
                }
                if (!crashUploadAttempted) {
                    crashUploadAttempted = true
                    scope.launch {
                        try {
                            client.uploadPendingCrash(mutableState.value.settings)
                        } catch (_: Throwable) {
                        }
                    }
                }
            }
            is Response -> {
                val action = pending.remove(message.requestId)
                if (!message.ok) {
                    if (action is PendingAction.Open) openedSessions.remove(action.sessionId)
                    mutableState.update {
                        it.copy(
                            drafts = if (action is PendingAction.Prompt
                                && it.drafts[action.sessionId].isNullOrEmpty()
                            ) {
                                it.drafts + (action.sessionId to action.text)
                            } else {
                                it.drafts
                            },
                            attachments = if (action is PendingAction.Prompt) {
                                val existing = it.attachments[action.sessionId].orEmpty()
                                val restored = action.files.filter { file ->
                                    existing.none { it === file }
                                } + existing
                                if (restored.isEmpty()) {
                                    it.attachments
                                } else {
                                    it.attachments + (action.sessionId to restored)
                                }
                            } else {
                                it.attachments
                            },
                            loadingCommands = if (action is PendingAction.Commands) {
                                it.loadingCommands - action.sessionId
                            } else {
                                it.loadingCommands
                            },
                            error = message.error ?: "Tau rejected the request.",
                        )
                    }
                } else if (action is PendingAction.Prompt) {
                    mutableState.update { current ->
                        if (message.commandHandled == true) {
                            current.copy(notice = message.notice ?: current.notice)
                        } else {
                            val outgoing = reconcileOutgoingMessages(
                                current.outgoingMessages[action.sessionId].orEmpty() +
                                    OutgoingMessage(
                                        requestId = message.requestId,
                                        text = action.displayText,
                                        afterEntryId = action.afterEntryId,
                                        occurrence = action.occurrence,
                                    ),
                                current.histories[action.sessionId].orEmpty(),
                            )
                            current.copy(
                                outgoingMessages = if (outgoing.isEmpty()) {
                                    current.outgoingMessages - action.sessionId
                                } else {
                                    current.outgoingMessages + (action.sessionId to outgoing)
                                },
                            )
                        }
                    }
                } else if (action is PendingAction.Commands) {
                    mutableState.update {
                        it.copy(loadingCommands = it.loadingCommands - action.sessionId)
                    }
                } else if (
                    (action == PendingAction.CreateSession || action == PendingAction.SelectSession) &&
                    message.sessionId != null
                ) {
                    val sessionId = message.sessionId
                    mutableState.update {
                        it.copy(
                            selectedSessionId = sessionId,
                            focusComposerSessionId = if (action == PendingAction.CreateSession) {
                                sessionId
                            } else {
                                it.focusComposerSessionId
                            },
                            mobileChatVisible = true,
                            drafts = if (message.draft != null) {
                                it.drafts + (sessionId to message.draft)
                            } else {
                                it.drafts
                            },
                            error = null,
                        )
                    }
                    openSession(sessionId, true)
                }
            }
            is Commands -> {
                mutableState.update {
                    it.copy(
                        slashCommands = it.slashCommands + (message.sessionId to message.commands),
                        loadingCommands = it.loadingCommands - message.sessionId,
                    )
                }
            }
            is ExtensionUi -> {
                val sessionId = message.sessionId
                val request = message.request
                mutableState.update { current ->
                    when (request.method) {
                        "select", "confirm", "input", "editor" -> {
                            val dialog = SessionExtensionUi(sessionId, request)
                            if (current.extensionDialogs.any { active ->
                                    active.sessionId == sessionId && active.request.id == request.id
                                }
                            ) {
                                current
                            } else {
                                current.copy(extensionDialogs = current.extensionDialogs + dialog)
                            }
                        }
                        "notify" -> if (request.notifyType == "error") {
                            current.copy(error = request.message ?: "Pi extension failed.")
                        } else {
                            current.copy(notice = request.message)
                        }
                        "setStatus" -> {
                            val statuses = current.extensionStatuses[sessionId].orEmpty()
                            val updated = if (request.statusKey == null) {
                                statuses
                            } else if (request.statusText == null) {
                                statuses - request.statusKey
                            } else {
                                statuses + (request.statusKey to request.statusText)
                            }
                            current.copy(
                                extensionStatuses = if (updated.isEmpty()) {
                                    current.extensionStatuses - sessionId
                                } else {
                                    current.extensionStatuses + (sessionId to updated)
                                },
                            )
                        }
                        "setWidget" -> {
                            val widgets = current.extensionWidgets[sessionId].orEmpty()
                            val updated = if (request.widgetKey == null) {
                                widgets
                            } else if (request.widgetLines.isEmpty()) {
                                widgets - request.widgetKey
                            } else {
                                widgets + (request.widgetKey to ExtensionWidget(
                                    lines = request.widgetLines,
                                    placement = request.widgetPlacement,
                                ))
                            }
                            current.copy(
                                extensionWidgets = if (updated.isEmpty()) {
                                    current.extensionWidgets - sessionId
                                } else {
                                    current.extensionWidgets + (sessionId to updated)
                                },
                            )
                        }
                        "set_editor_text" -> current.copy(
                            drafts = current.drafts + (sessionId to request.text.orEmpty()),
                        )
                        "setTitle" -> current
                        else -> current.copy(
                            error = "Pi requested unsupported extension UI: ${request.method}",
                        )
                    }
                }
            }
            is ExtensionError -> {
                mutableState.update { it.copy(error = message.error) }
            }
            is Sessions -> {
                val activeIds = message.sessions.mapTo(mutableSetOf(), SessionSummary::id)
                downloadJobs.keys
                    .filter { key -> key.sessionId !in activeIds }
                    .forEach { key -> downloadJobs.remove(key)?.cancel() }
                val currentSelection = mutableState.value.selectedSessionId
                val selected = currentSelection
                    ?.takeIf { it in activeIds }
                    ?: message.sessions.firstOrNull()?.id
                openedSessions.retainAll(activeIds)
                mutableState.update {
                    it.copy(
                        sessions = message.sessions,
                        selectedSessionId = selected,
                        focusComposerSessionId = it.focusComposerSessionId
                            ?.takeIf { sessionId -> sessionId in activeIds },
                        histories = it.histories.filterKeys { sessionId -> sessionId in activeIds },
                        outgoingMessages = it.outgoingMessages.filterKeys { sessionId ->
                            sessionId in activeIds
                        },
                        partials = it.partials.filterKeys { sessionId -> sessionId in activeIds },
                        drafts = it.drafts.filterKeys { sessionId -> sessionId in activeIds },
                        attachments = it.attachments.filterKeys { sessionId -> sessionId in activeIds },
                        slashCommands = it.slashCommands.filterKeys { sessionId -> sessionId in activeIds },
                        loadingCommands = it.loadingCommands.filterTo(mutableSetOf()) { sessionId ->
                            sessionId in activeIds
                        },
                        extensionDialogs = it.extensionDialogs.filter { dialog ->
                            dialog.sessionId in activeIds
                        },
                        extensionStatuses = it.extensionStatuses.filterKeys { sessionId ->
                            sessionId in activeIds
                        },
                        extensionWidgets = it.extensionWidgets.filterKeys { sessionId ->
                            sessionId in activeIds
                        },
                        attachmentDownloads = it.attachmentDownloads.filterKeys { key ->
                            key.sessionId in activeIds
                        },
                        uploadingSessions = it.uploadingSessions.filterTo(mutableSetOf()) { sessionId ->
                            sessionId in activeIds
                        },
                        mobileChatVisible = it.mobileChatVisible && selected != null,
                    )
                }
                if (selected != null && selected !in openedSessions) openSession(selected, false)
            }
            is History -> {
                mutableState.update { current ->
                    val outgoing = reconcileOutgoingMessages(
                        current.outgoingMessages[message.sessionId].orEmpty(),
                        message.messages,
                    )
                    current.copy(
                        histories = current.histories + (message.sessionId to message.messages),
                        outgoingMessages = if (outgoing.isEmpty()) {
                            current.outgoingMessages - message.sessionId
                        } else {
                            current.outgoingMessages + (message.sessionId to outgoing)
                        },
                        partials = current.partials - message.sessionId,
                    )
                }
            }
            is SessionState -> {
                mutableState.update { current ->
                    val processStopped = message.status == SessionStatus.Sleeping ||
                        message.status == SessionStatus.Error
                    current.copy(
                        sessions = current.sessions.map { session ->
                            if (session.id == message.sessionId) {
                                session.copy(status = message.status, detail = message.detail)
                            } else {
                                session
                            }
                        },
                        slashCommands = if (processStopped) {
                            current.slashCommands - message.sessionId
                        } else {
                            current.slashCommands
                        },
                        loadingCommands = if (processStopped) {
                            current.loadingCommands - message.sessionId
                        } else {
                            current.loadingCommands
                        },
                        extensionDialogs = if (processStopped) {
                            current.extensionDialogs.filterNot { dialog ->
                                dialog.sessionId == message.sessionId
                            }
                        } else {
                            current.extensionDialogs
                        },
                        extensionStatuses = if (processStopped) {
                            current.extensionStatuses - message.sessionId
                        } else {
                            current.extensionStatuses
                        },
                        extensionWidgets = if (processStopped) {
                            current.extensionWidgets - message.sessionId
                        } else {
                            current.extensionWidgets
                        },
                    )
                }
            }
            is StreamReset -> {
                mutableState.update {
                    it.copy(partials = it.partials + (message.sessionId to ""))
                }
            }
            is StreamDelta -> {
                mutableState.update {
                    it.copy(
                        partials = it.partials +
                            (message.sessionId to (it.partials[message.sessionId].orEmpty() + message.delta)),
                    )
                }
            }
            is StreamEnd -> Unit
            ResyncRequired -> {
                openedSessions.clear()
                val id = nextRequestId()
                pending[id] = PendingAction.Normal
                client.send(ListSessions(id))
                mutableState.value.selectedSessionId?.let { openSession(it, true) }
            }
        }
    }

    private fun openSession(sessionId: String, force: Boolean) {
        if (!force && sessionId in openedSessions) return
        openedSessions += sessionId
        val id = nextRequestId()
        pending[id] = PendingAction.Open(sessionId)
        send(OpenSession(id, sessionId))
    }

    private fun send(request: ClientRequest) {
        scope.launch {
            try {
                client.send(request)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                val action = pending.remove(request.id)
                if (action is PendingAction.Open) openedSessions.remove(action.sessionId)
                mutableState.update {
                    val loadingCommands = if (action is PendingAction.Commands) {
                        it.loadingCommands - action.sessionId
                    } else {
                        it.loadingCommands
                    }
                    val dialogs = if (
                        action is PendingAction.ExtensionUi &&
                        error !is TauConnectionException &&
                        it.extensionDialogs.none { active ->
                            active.sessionId == action.dialog.sessionId &&
                                active.request.id == action.dialog.request.id
                        }
                    ) {
                        it.extensionDialogs + action.dialog
                    } else {
                        it.extensionDialogs
                    }
                    if (error is TauConnectionException) {
                        it.copy(
                            connectionStatus = ConnectionStatus.Offline,
                            loadingCommands = loadingCommands,
                            extensionDialogs = dialogs,
                        )
                    } else {
                        it.copy(
                            loadingCommands = loadingCommands,
                            extensionDialogs = dialogs,
                            error = error.message ?: "Tau request was not sent.",
                        )
                    }
                }
            }
        }
    }

    private fun nextRequestId(): String = "client-${requestSequence++}"

    private sealed interface PendingAction {
        data object Normal : PendingAction
        data object CreateSession : PendingAction
        data object SelectSession : PendingAction
        data class Open(val sessionId: String) : PendingAction
        data class Commands(val sessionId: String) : PendingAction
        data class ExtensionUi(val dialog: SessionExtensionUi) : PendingAction
        data class Prompt(
            val sessionId: String,
            val text: String,
            val files: List<PickedFile>,
            val displayText: String,
            val afterEntryId: String?,
            val occurrence: Int,
        ) : PendingAction
    }
}

internal fun reconcileOutgoingMessages(
    outgoing: List<OutgoingMessage>,
    history: List<ChatMessage>,
): List<OutgoingMessage> = outgoing.filter { pending ->
    val start = if (pending.afterEntryId == null) {
        0
    } else {
        val baseline = history.indexOfFirst { message -> message.entryId == pending.afterEntryId }
        if (baseline < 0) return@filter true
        baseline + 1
    }
    history
        .asSequence()
        .drop(start)
        .filter { message -> message.role == ChatRole.User }
        .drop(pending.occurrence)
        .none()
}
