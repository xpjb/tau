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
    val canonicalText: String,
    val afterEntryId: String?,
    val occurrence: Int,
    val canonicalOccurrence: Int,
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

data class DetailExpansionKey(val sessionId: String, val entryId: String)

enum class DetailContentKind {
    Tool,
    Arguments,
    Result,
}

data class DetailContentExpansionKey(
    val sessionId: String,
    val entryId: String,
    val detailIndex: Int,
    val content: DetailContentKind,
)

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
    val liveAttempts: Map<String, List<ChatAttempt>> = emptyMap(),
    val detailsExpandedBySession: Map<String, Boolean> = emptyMap(),
    val detailExpansions: Map<DetailExpansionKey, Boolean> = emptyMap(),
    val expandedDetailContent: Set<DetailContentExpansionKey> = emptySet(),
    val messageDetails: Map<DetailExpansionKey, MessageDetails> = emptyMap(),
    val loadingMessageDetails: Set<DetailExpansionKey> = emptySet(),
    val messageDetailErrors: Map<DetailExpansionKey, String> = emptyMap(),
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
    private val detailJobs = mutableMapOf<DetailExpansionKey, Job>()
    private val detailContentJobs = mutableMapOf<DetailContentExpansionKey, Job>()
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

    fun setDetailsExpanded(sessionId: String, entryId: String?, expanded: Boolean) {
        val key = entryId?.let { DetailExpansionKey(sessionId, it) }
        if (!expanded && key != null) {
            detailJobs.remove(key)?.cancel()
            detailContentJobs.keys
                .filter { content ->
                    content.sessionId == sessionId && content.entryId == entryId
                }
                .forEach { content -> detailContentJobs.remove(content)?.cancel() }
        }
        var loadDetails = false
        mutableState.update { current ->
            val previousDefault = current.detailsExpandedBySession[sessionId] ?: false
            val expansions = current.detailExpansions.toMutableMap()
            current.histories[sessionId]
                .orEmpty()
                .filter(ChatMessage::hasDetails)
                .forEach { message ->
                    expansions.putIfAbsent(
                        DetailExpansionKey(sessionId, message.groupId ?: message.entryId),
                        previousDefault,
                    )
                }
            val message = entryId?.let { id ->
                current.histories[sessionId]
                    .orEmpty()
                    .firstOrNull { message ->
                        (message.groupId ?: message.entryId) == id
                    }
            }
            val loadedDetails = key?.let(current.messageDetails::get)
            val detailsStale = message != null && loadedDetails != null &&
                message.attempts.any { attempt ->
                    loadedDetails.attempts.none { loaded -> loaded.entryId == attempt.entryId }
                }
            loadDetails = expanded && key != null && message?.hasDetails == true &&
                (loadedDetails == null || detailsStale) &&
                key !in current.loadingMessageDetails &&
                key !in current.messageDetailErrors
            val waitingForDetails = expanded && key != null && loadedDetails == null &&
                (loadDetails || key in current.loadingMessageDetails)
            if (key != null) {
                expansions[key] = expanded && !waitingForDetails
            }
            current.copy(
                detailsExpandedBySession = current.detailsExpandedBySession +
                    (sessionId to expanded),
                detailExpansions = expansions,
                expandedDetailContent = if (!expanded && entryId != null) {
                    current.expandedDetailContent.filterTo(mutableSetOf()) { content ->
                        content.sessionId != sessionId || content.entryId != entryId
                    }
                } else {
                    current.expandedDetailContent
                },
                messageDetails = current.messageDetails,
                loadingMessageDetails = when {
                    loadDetails -> current.loadingMessageDetails + checkNotNull(key)
                    !expanded && key != null -> current.loadingMessageDetails - key
                    else -> current.loadingMessageDetails
                },
                messageDetailErrors = if (key != null) {
                    current.messageDetailErrors - key
                } else {
                    current.messageDetailErrors
                },
            )
        }
        if (!loadDetails || key == null) return
        lateinit var job: Job
        job = scope.launch(start = CoroutineStart.LAZY) {
            try {
                val details = client.messageDetails(
                    mutableState.value.settings,
                    key.sessionId,
                    key.entryId,
                )
                mutableState.update { current ->
                    if (key !in current.loadingMessageDetails || detailJobs[key] !== job) {
                        current
                    } else {
                        current.copy(
                            detailExpansions = current.detailExpansions + (key to true),
                            messageDetails = current.messageDetails + (
                                key to details.copy(
                                    attempts = details.attempts.map { attempt ->
                                        current.messageDetails[key]
                                            ?.attempts
                                            ?.firstOrNull { prior ->
                                                prior.entryId == attempt.entryId
                                            }
                                            ?.let(attempt::withMissingContentFrom)
                                            ?: attempt
                                    },
                                )
                            ),
                            loadingMessageDetails = current.loadingMessageDetails - key,
                            messageDetailErrors = current.messageDetailErrors - key,
                        )
                    }
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                mutableState.update { current ->
                    if (key !in current.loadingMessageDetails || detailJobs[key] !== job) {
                        current
                    } else {
                        current.copy(
                            detailExpansions = current.detailExpansions + (key to true),
                            loadingMessageDetails = current.loadingMessageDetails - key,
                            messageDetailErrors = current.messageDetailErrors +
                                (key to (error.message ?: "Details could not be loaded.").take(240)),
                        )
                    }
                }
            } finally {
                if (detailJobs[key] === job) {
                    detailJobs.remove(key)
                    mutableState.update { current ->
                        current.copy(
                            loadingMessageDetails = current.loadingMessageDetails - key,
                        )
                    }
                }
            }
        }
        detailJobs[key] = job
        job.start()
    }

    fun toggleDetailContent(key: DetailContentExpansionKey) {
        val current = mutableState.value
        if (key in current.expandedDetailContent) {
            detailContentJobs.remove(key)?.cancel()
            mutableState.update {
                it.copy(expandedDetailContent = it.expandedDetailContent - key)
            }
            return
        }
        val messageKey = DetailExpansionKey(key.sessionId, key.entryId)
        val content = current.messageDetails[messageKey]
            ?.attempts
            ?.asSequence()
            ?.flatMap { attempt -> attempt.content.asSequence() }
            ?.firstOrNull { content -> content.detailIndex == key.detailIndex }
        val needsToolContent = key.content == DetailContentKind.Tool &&
            content?.kind == ChatContentKind.Tool &&
            ((content.hasArguments && content.arguments == null) ||
                (content.hasResult && content.result == null))
        if (!needsToolContent) {
            mutableState.update {
                it.copy(expandedDetailContent = it.expandedDetailContent + key)
            }
            return
        }
        if (key in detailContentJobs) return

        lateinit var job: Job
        job = scope.launch(start = CoroutineStart.LAZY) {
            val loaded = try {
                client.messageDetail(
                    mutableState.value.settings,
                    key.sessionId,
                    key.entryId,
                    key.detailIndex,
                )
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                ChatDetail(
                    kind = ChatDetailKind.Tool,
                    toolName = content.toolName,
                    result = error.message ?: "Tool details could not be loaded.",
                    hasArguments = content.hasArguments,
                    hasResult = true,
                    isError = true,
                )
            }
            mutableState.update { state ->
                if (detailContentJobs[key] !== job) {
                    state
                } else {
                    val details = state.messageDetails[messageKey]
                    if (details == null) {
                        state
                    } else {
                        state.copy(
                            expandedDetailContent = state.expandedDetailContent + key,
                            messageDetails = state.messageDetails + (
                                messageKey to MessageDetails(
                                    attempts = details.attempts.map { attempt ->
                                        attempt.copy(
                                            content = attempt.content.map { content ->
                                                if (content.detailIndex == key.detailIndex) {
                                                    content.copy(
                                                        toolName = loaded.toolName,
                                                        arguments = loaded.arguments,
                                                        result = loaded.result,
                                                        hasArguments = loaded.hasArguments,
                                                        hasResult = loaded.hasResult,
                                                        isError = loaded.isError,
                                                    )
                                                } else {
                                                    content
                                                }
                                            },
                                        )
                                    },
                                )
                            ),
                        )
                    }
                }
            }
            if (detailContentJobs[key] === job) detailContentJobs.remove(key)
        }
        detailContentJobs[key] = job
        job.start()
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
                val current = mutableState.value
                val history = current.histories[sessionId].orEmpty()
                val afterEntryId = history.lastOrNull()?.entryId
                val occurrence = current.outgoingMessages[sessionId]
                    .orEmpty()
                    .count { outgoing -> outgoing.afterEntryId == afterEntryId } +
                    pending.values.count { action ->
                        action is PendingAction.Prompt &&
                            action.sessionId == sessionId &&
                            action.afterEntryId == afterEntryId
                    }
                val canonicalOccurrence = history.count { canonical ->
                    canonical.role == ChatRole.User && canonical.text == message
                } + current.outgoingMessages[sessionId]
                    .orEmpty()
                    .count { outgoing -> outgoing.canonicalText == message } +
                    pending.values.count { action ->
                        action is PendingAction.Prompt &&
                            action.sessionId == sessionId &&
                            action.canonicalText == message
                    }
                pending[id] = PendingAction.Prompt(
                    sessionId = sessionId,
                    text = text,
                    files = files,
                    displayText = introduction,
                    canonicalText = message,
                    afterEntryId = afterEntryId,
                    occurrence = occurrence,
                    canonicalOccurrence = canonicalOccurrence,
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
                it.copy(error = error.message ?: "The downloaded file could not be opened.")
            }
        }
    }

    fun showAttachmentDownload(message: ChatMessage) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val key = AttachmentDownloadKey(sessionId, message.entryId)
        val download = mutableState.value.attachmentDownloads[key]?.saved ?: return
        try {
            PlatformServices.showDownload(download)
        } catch (error: Throwable) {
            mutableState.update {
                it.copy(error = error.message ?: "The downloaded file could not be shown.")
            }
        }
    }

    fun extractAndOpenAttachmentDownload(message: ChatMessage) {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val key = AttachmentDownloadKey(sessionId, message.entryId)
        val download = mutableState.value.attachmentDownloads[key]?.saved ?: return
        try {
            PlatformServices.extractAndOpenDownload(download)
        } catch (error: Throwable) {
            mutableState.update {
                it.copy(error = error.message ?: "The downloaded ZIP could not be extracted.")
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

    private fun appendLiveDelta(
        sessionId: String,
        attemptId: String,
        contentIndex: Int,
        kind: ChatContentKind,
        delta: String,
    ) {
        updateLiveContent(sessionId, attemptId, contentIndex) { previous ->
            if (previous?.kind == kind) {
                previous.copy(text = previous.text.orEmpty() + delta)
            } else {
                ChatContent(
                    kind = kind,
                    contentIndex = contentIndex,
                    detailIndex = null,
                    text = delta,
                    hasContent = true,
                )
            }
        }
    }

    private fun updateLiveContent(
        sessionId: String,
        attemptId: String,
        contentIndex: Int,
        update: (ChatContent?) -> ChatContent,
    ) {
        mutableState.update { current ->
            val attempts = current.liveAttempts[sessionId].orEmpty()
            val attemptIndex = attempts.indexOfFirst { attempt -> attempt.entryId == attemptId }
            if (attemptIndex < 0) return@update current
            val attempt = attempts[attemptIndex]
            val previous = attempt.content.firstOrNull { content ->
                content.contentIndex == contentIndex
            }
            val content = attempt.content
                .filterNot { existing -> existing.contentIndex == contentIndex }
                .plus(update(previous))
                .sortedBy(ChatContent::contentIndex)
            val updatedAttempts = attempts.toMutableList().also { list ->
                list[attemptIndex] = attempt.copy(content = content)
            }
            current.copy(
                liveAttempts = current.liveAttempts + (sessionId to updatedAttempts),
            )
        }
    }

    private fun connect(settings: ConnectionSettings) {
        connectionJob?.cancel()
        downloadJobs.values.forEach { job -> job.cancel() }
        downloadJobs.clear()
        detailJobs.values.forEach { job -> job.cancel() }
        detailJobs.clear()
        detailContentJobs.values.forEach { job -> job.cancel() }
        detailContentJobs.clear()
        openedSessions.clear()
        pending.clear()
        mutableState.update {
            it.copy(
                attachmentDownloads = emptyMap(),
                messageDetails = emptyMap(),
                loadingMessageDetails = emptySet(),
                messageDetailErrors = emptyMap(),
            )
        }
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
                                        canonicalText = action.canonicalText,
                                        afterEntryId = action.afterEntryId,
                                        occurrence = action.occurrence,
                                        canonicalOccurrence = action.canonicalOccurrence,
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
                detailJobs.keys
                    .filter { key -> key.sessionId !in activeIds }
                    .forEach { key -> detailJobs.remove(key)?.cancel() }
                detailContentJobs.keys
                    .filter { key -> key.sessionId !in activeIds }
                    .forEach { key -> detailContentJobs.remove(key)?.cancel() }
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
                        liveAttempts = it.liveAttempts.filterKeys { sessionId ->
                            sessionId in activeIds
                        },
                        detailsExpandedBySession = it.detailsExpandedBySession.filterKeys {
                            sessionId -> sessionId in activeIds
                        },
                        detailExpansions = it.detailExpansions.filterKeys { key ->
                            key.sessionId in activeIds
                        },
                        expandedDetailContent = it.expandedDetailContent.filterTo(mutableSetOf()) {
                            key -> key.sessionId in activeIds
                        },
                        messageDetails = it.messageDetails.filterKeys { key ->
                            key.sessionId in activeIds
                        },
                        loadingMessageDetails = it.loadingMessageDetails.filterTo(mutableSetOf()) {
                            key -> key.sessionId in activeIds
                        },
                        messageDetailErrors = it.messageDetailErrors.filterKeys { key ->
                            key.sessionId in activeIds
                        },
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
                val activeEntries = message.messages.flatMapTo(mutableSetOf()) { chatMessage ->
                    listOfNotNull(chatMessage.entryId, chatMessage.groupId)
                }
                val canonicalAttemptTimestamps = message.messages
                    .asSequence()
                    .flatMap { chatMessage -> chatMessage.attempts.asSequence() }
                    .mapNotNull(ChatAttempt::timestampMs)
                    .toSet()
                detailJobs.keys
                    .filter { key ->
                        key.sessionId == message.sessionId && key.entryId !in activeEntries
                    }
                    .forEach { key -> detailJobs.remove(key)?.cancel() }
                detailContentJobs.keys
                    .filter { key ->
                        key.sessionId == message.sessionId && key.entryId !in activeEntries
                    }
                    .forEach { key -> detailContentJobs.remove(key)?.cancel() }
                mutableState.update { current ->
                    val messages = preserveAttemptContent(
                        incoming = message.messages,
                        previous = current.histories[message.sessionId].orEmpty(),
                        live = current.liveAttempts[message.sessionId].orEmpty(),
                    )
                    val outgoing = reconcileOutgoingMessages(
                        current.outgoingMessages[message.sessionId].orEmpty(),
                        messages,
                    )
                    current.copy(
                        histories = current.histories + (message.sessionId to messages),
                        outgoingMessages = if (outgoing.isEmpty()) {
                            current.outgoingMessages - message.sessionId
                        } else {
                            current.outgoingMessages + (message.sessionId to outgoing)
                        },
                        liveAttempts = current.liveAttempts[message.sessionId]
                            .orEmpty()
                            .filterNot { attempt ->
                                attempt.timestampMs != null &&
                                    attempt.timestampMs in canonicalAttemptTimestamps
                            }
                            .let { attempts ->
                                if (attempts.isEmpty()) {
                                    current.liveAttempts - message.sessionId
                                } else {
                                    current.liveAttempts + (message.sessionId to attempts)
                                }
                            },
                        detailExpansions = current.detailExpansions.filterKeys { key ->
                            key.sessionId != message.sessionId || key.entryId in activeEntries
                        },
                        expandedDetailContent = current.expandedDetailContent.filterTo(
                            mutableSetOf(),
                        ) { key ->
                            key.sessionId != message.sessionId || key.entryId in activeEntries
                        },
                        messageDetails = current.messageDetails.filterKeys { key ->
                            key.sessionId != message.sessionId || key.entryId in activeEntries
                        },
                        loadingMessageDetails = current.loadingMessageDetails.filterTo(
                            mutableSetOf(),
                        ) { key ->
                            key.sessionId != message.sessionId || key.entryId in activeEntries
                        },
                        messageDetailErrors = current.messageDetailErrors.filterKeys { key ->
                            key.sessionId != message.sessionId || key.entryId in activeEntries
                        },
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
            is StreamReset -> mutableState.update { current ->
                val attempts = current.liveAttempts[message.sessionId].orEmpty()
                if (attempts.any { attempt -> attempt.entryId == message.attemptId }) {
                    current
                } else {
                    current.copy(
                        liveAttempts = current.liveAttempts + (
                            message.sessionId to attempts + ChatAttempt(
                                entryId = message.attemptId,
                                timestampMs = message.timestampMs,
                            )
                        ),
                    )
                }
            }
            is StreamDelta -> appendLiveDelta(
                message.sessionId,
                message.attemptId,
                message.contentIndex,
                ChatContentKind.Text,
                message.delta,
            )
            is StreamDetailsDelta -> appendLiveDelta(
                message.sessionId,
                message.attemptId,
                message.contentIndex,
                ChatContentKind.Thinking,
                message.delta,
            )
            is StreamTool -> updateLiveContent(
                message.sessionId,
                message.attemptId,
                message.contentIndex,
            ) {
                ChatContent(
                    kind = ChatContentKind.Tool,
                    contentIndex = message.contentIndex,
                    detailIndex = null,
                    toolName = message.toolName,
                    arguments = message.arguments,
                    hasContent = true,
                    hasArguments = message.arguments != null,
                )
            }
            is StreamSnapshot -> {
                mutableState.update { current ->
                    current.copy(
                        liveAttempts = if (message.attempts.isEmpty()) {
                            current.liveAttempts - message.sessionId
                        } else {
                            current.liveAttempts + (message.sessionId to message.attempts)
                        },
                    )
                }
            }
            is StreamEnd -> {
                message.attempt?.let { completed ->
                    mutableState.update { current ->
                        val attempts = current.liveAttempts[message.sessionId].orEmpty()
                        val index = attempts.indexOfFirst { attempt ->
                            attempt.entryId == completed.entryId
                        }
                        val updated = if (index < 0) {
                            attempts + completed
                        } else {
                            attempts.toMutableList().also { it[index] = completed }
                        }
                        current.copy(
                            liveAttempts = current.liveAttempts + (message.sessionId to updated),
                        )
                    }
                }
            }
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
            val canonicalText: String,
            val afterEntryId: String?,
            val occurrence: Int,
            val canonicalOccurrence: Int,
        ) : PendingAction
    }
}

private fun ChatAttempt.withMissingContentFrom(source: ChatAttempt): ChatAttempt = copy(
    content = content.map { content ->
        val prior = source.content.firstOrNull { candidate ->
            candidate.contentIndex == content.contentIndex && candidate.kind == content.kind
        } ?: return@map content
        content.copy(
            text = content.text ?: prior.text,
            toolName = content.toolName ?: prior.toolName,
            arguments = content.arguments ?: prior.arguments,
            result = content.result ?: prior.result,
            hasContent = content.hasContent || prior.hasContent,
            hasArguments = content.hasArguments || prior.hasArguments,
            hasResult = content.hasResult || prior.hasResult,
            isError = content.isError || prior.isError,
        )
    },
)

internal fun preserveAttemptContent(
    incoming: List<ChatMessage>,
    previous: List<ChatMessage>,
    live: List<ChatAttempt>,
): List<ChatMessage> {
    val previousAttempts = previous
        .asSequence()
        .flatMap { message -> message.attempts.asSequence() }
        .associateBy(ChatAttempt::entryId)
    val liveByTimestamp = live.mapNotNull { attempt ->
        attempt.timestampMs?.let { timestamp -> timestamp to attempt }
    }.toMap()
    return incoming.map { message ->
        message.copy(
            attempts = message.attempts.map attemptMap@ { attempt ->
                val source = previousAttempts[attempt.entryId]
                    ?: attempt.timestampMs?.let(liveByTimestamp::get)
                    ?: return@attemptMap attempt
                attempt.withMissingContentFrom(source)
            },
        )
    }
}

internal fun mergeLiveAttempts(
    messages: List<ChatMessage>,
    liveAttempts: List<ChatAttempt>,
): List<ChatMessage> {
    if (liveAttempts.isEmpty()) return messages
    val result = messages.toMutableList()
    val last = result.lastOrNull()
    val lastAttempt = last?.attempts?.lastOrNull()
    val mergeWithLast = last?.role == ChatRole.Assistant && lastAttempt != null &&
        lastAttempt.stopReason in setOf(null, "toolUse", "error", "aborted")
    val liveText = liveAttempts
        .flatMap(ChatAttempt::content)
        .filter { content -> content.kind == ChatContentKind.Text }
        .mapNotNull(ChatContent::text)
        .filter(String::isNotEmpty)
    if (mergeWithLast) {
        result[result.lastIndex] = checkNotNull(last).copy(
            text = (listOf(last.text) + liveText)
                .filter(String::isNotEmpty)
                .joinToString("\n\n"),
            attempts = last.attempts + liveAttempts,
        )
        return result
    }
    val timestamp = liveAttempts.first().timestampMs
    val message = ChatMessage(
        entryId = "live-${liveAttempts.first().entryId}",
        role = ChatRole.Assistant,
        text = liveText.joinToString("\n\n"),
        timestampMs = timestamp,
        attempts = liveAttempts,
    )
    val insertion = timestamp?.let { liveTimestamp ->
        result.indexOfFirst { canonical ->
            canonical.timestampMs?.let { it > liveTimestamp } == true
        }
    } ?: -1
    if (insertion < 0) result += message else result.add(insertion, message)
    return result
}

internal fun reconcileOutgoingMessages(
    outgoing: List<OutgoingMessage>,
    history: List<ChatMessage>,
): List<OutgoingMessage> = outgoing.filter { pending ->
    val acceptedByContent = history
        .asSequence()
        .filter { message ->
            message.role == ChatRole.User && message.text == pending.canonicalText
        }
        .drop(pending.canonicalOccurrence)
        .any()
    if (acceptedByContent) return@filter false

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
