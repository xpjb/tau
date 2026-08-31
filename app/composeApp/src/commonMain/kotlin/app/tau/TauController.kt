package app.tau

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
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

private const val ReconnectDelayMillis = 2_000L

enum class ConnectionStatus {
    NotConfigured,
    Connecting,
    Connected,
    Offline,
}

data class TauUiState(
    val settings: ConnectionSettings = ConnectionSettings(),
    val editingSettings: Boolean = false,
    val connectionStatus: ConnectionStatus = ConnectionStatus.NotConfigured,
    val daemonVersion: String? = null,
    val sessions: List<SessionSummary> = emptyList(),
    val selectedSessionId: String? = null,
    val histories: Map<String, List<ChatMessage>> = emptyMap(),
    val partials: Map<String, String> = emptyMap(),
    val drafts: Map<String, String> = emptyMap(),
    val mobileChatVisible: Boolean = false,
    val error: String? = null,
)

class TauController {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val client = TauClient()
    private val mutableState = MutableStateFlow(TauUiState())
    private val pending = mutableMapOf<String, PendingAction>()
    private val openedSessions = mutableSetOf<String>()
    private var connectionJob: Job? = null
    private var requestSequence = 1L
    private var crashUploadAttempted = false

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
        pending[id] = PendingAction.SelectSession
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

    fun setDraft(sessionId: String, draft: String) {
        mutableState.update { it.copy(drafts = it.drafts + (sessionId to draft)) }
    }

    fun sendPrompt() {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val text = mutableState.value.drafts[sessionId].orEmpty()
        if (text.isBlank()) return
        val id = nextRequestId()
        pending[id] = PendingAction.Normal
        scope.launch {
            try {
                client.send(Prompt(id, sessionId, text))
                mutableState.update { current ->
                    if (current.drafts[sessionId] == text) {
                        current.copy(drafts = current.drafts + (sessionId to ""), error = null)
                    } else {
                        current.copy(error = null)
                    }
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                pending.remove(id)
                mutableState.update { it.copy(error = error.message ?: "Message was not sent.") }
            }
        }
    }

    fun abort() {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val id = nextRequestId()
        pending[id] = PendingAction.Normal
        send(Abort(id, sessionId))
    }

    fun closeSession() {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val id = nextRequestId()
        pending[id] = PendingAction.Normal
        send(CloseSession(id, sessionId))
    }

    fun renameSession(title: String) {
        val sessionId = mutableState.value.selectedSessionId ?: return
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

    fun cloneSession() {
        val sessionId = mutableState.value.selectedSessionId ?: return
        val id = nextRequestId()
        pending[id] = PendingAction.SelectSession
        send(CloneSession(id, sessionId))
    }

    fun dismissError() {
        mutableState.update { it.copy(error = null) }
    }

    fun dispose() {
        connectionJob?.cancel()
        client.close()
        scope.cancel()
    }

    private fun connect(settings: ConnectionSettings) {
        connectionJob?.cancel()
        openedSessions.clear()
        pending.clear()
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
                    mutableState.update {
                        it.copy(
                            connectionStatus = ConnectionStatus.Offline,
                            error = error.message?.take(240) ?: "Tau connection failed.",
                        )
                    }
                }
                openedSessions.clear()
                mutableState.update {
                    it.copy(connectionStatus = ConnectionStatus.Offline, daemonVersion = null)
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
                        it.copy(error = message.error ?: "Tau rejected the request.")
                    }
                } else if (action == PendingAction.SelectSession && message.sessionId != null) {
                    val sessionId = message.sessionId
                    mutableState.update {
                        it.copy(
                            selectedSessionId = sessionId,
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
            is Sessions -> {
                val currentSelection = mutableState.value.selectedSessionId
                val selected = currentSelection
                    ?.takeIf { selectedId -> message.sessions.any { it.id == selectedId } }
                    ?: message.sessions.firstOrNull()?.id
                mutableState.update {
                    it.copy(sessions = message.sessions, selectedSessionId = selected)
                }
                if (selected != null && selected !in openedSessions) openSession(selected, false)
            }
            is History -> {
                mutableState.update {
                    it.copy(
                        histories = it.histories + (message.sessionId to message.messages),
                        partials = it.partials - message.sessionId,
                    )
                }
            }
            is SessionState -> {
                mutableState.update { current ->
                    current.copy(
                        sessions = current.sessions.map { session ->
                            if (session.id == message.sessionId) {
                                session.copy(status = message.status, detail = message.detail)
                            } else {
                                session
                            }
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
                    it.copy(error = error.message ?: "Tau request was not sent.")
                }
            }
        }
    }

    private fun nextRequestId(): String = "client-${requestSequence++}"

    private sealed interface PendingAction {
        data object Normal : PendingAction
        data object SelectSession : PendingAction
        data class Open(val sessionId: String) : PendingAction
    }
}
