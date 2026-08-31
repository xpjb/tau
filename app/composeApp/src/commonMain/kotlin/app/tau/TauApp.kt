package app.tau

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.displayCutoutPadding
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Snackbar
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.VerticalDivider
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.collectAsState

private val TauDarkColors = darkColorScheme(
    primary = Color(0xFF67D4FF),
    onPrimary = Color(0xFF003546),
    primaryContainer = Color(0xFF164E63),
    onPrimaryContainer = Color(0xFFC7F0FF),
    secondary = Color(0xFFA5B4FC),
    secondaryContainer = Color(0xFF303A66),
    tertiary = Color(0xFF5EEAD4),
    tertiaryContainer = Color(0xFF134E4A),
    background = Color(0xFF090D12),
    onBackground = Color(0xFFE5EAF0),
    surface = Color(0xFF0E141B),
    onSurface = Color(0xFFE5EAF0),
    surfaceVariant = Color(0xFF18212B),
    onSurfaceVariant = Color(0xFFB7C2CE),
    outline = Color(0xFF526170),
    outlineVariant = Color(0xFF2A3541),
    error = Color(0xFFFFB4AB),
    errorContainer = Color(0xFF7F1D1D),
)

@Composable
fun TauApp(controller: TauController) {
    val state by controller.state.collectAsState()
    DisposableEffect(controller) {
        controller.start()
        onDispose(controller::dispose)
    }

    MaterialTheme(colorScheme = TauDarkColors) {
        Surface(Modifier.fillMaxSize()) {
            if (state.editingSettings) {
                ConnectionScreen(state, controller)
            } else {
                Box(Modifier.fillMaxSize().systemBarsPadding().displayCutoutPadding()) {
                    BoxWithConstraints(Modifier.fillMaxSize()) {
                        if (maxWidth >= 760.dp) {
                            Row(Modifier.fillMaxSize()) {
                                SessionList(
                                    state = state,
                                    controller = controller,
                                    modifier = Modifier.width(300.dp).fillMaxHeight(),
                                )
                                VerticalDivider()
                                ChatPanel(
                                    state = state,
                                    controller = controller,
                                    showBack = false,
                                    modifier = Modifier.weight(1f).fillMaxHeight(),
                                )
                            }
                        } else if (state.mobileChatVisible && state.selectedSessionId != null) {
                            ChatPanel(
                                state = state,
                                controller = controller,
                                showBack = true,
                                modifier = Modifier.fillMaxSize(),
                            )
                        } else {
                            SessionList(
                                state = state,
                                controller = controller,
                                modifier = Modifier.fillMaxSize(),
                            )
                        }
                    }
                    (state.error ?: state.notice)?.let { message ->
                        Snackbar(
                            modifier = Modifier
                                .align(Alignment.BottomCenter)
                                .padding(16.dp)
                                .widthIn(max = 640.dp),
                            action = {
                                TextButton(
                                    onClick = if (state.error != null) {
                                        controller::dismissError
                                    } else {
                                        controller::dismissNotice
                                    },
                                ) { Text("Dismiss") }
                            },
                        ) {
                            Text(message)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ConnectionScreen(state: TauUiState, controller: TauController) {
    var serverUrl by remember(state.settings, state.editingSettings) {
        mutableStateOf(state.settings.serverUrl)
    }
    var token by remember(state.settings, state.editingSettings) {
        mutableStateOf(state.settings.token)
    }
    Box(
        Modifier.fillMaxSize().systemBarsPadding().displayCutoutPadding(),
        contentAlignment = Alignment.Center,
    ) {
        Card(Modifier.padding(24.dp).widthIn(max = 520.dp).fillMaxWidth()) {
            Column(
                Modifier.padding(24.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Text("Tau", style = MaterialTheme.typography.headlineLarge, fontWeight = FontWeight.Bold)
                Text("Connect directly to the Tau daemon over your Tailnet.")
                OutlinedTextField(
                    value = serverUrl,
                    onValueChange = { serverUrl = it },
                    label = { Text("Daemon URL") },
                    placeholder = { Text("http://vibe:8787") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = token,
                    onValueChange = { token = it },
                    label = { Text("Access token") },
                    visualTransformation = PasswordVisualTransformation(),
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp, Alignment.End),
                ) {
                    if (state.settings.token.isNotBlank()) {
                        TextButton(onClick = controller::hideSettings) { Text("Cancel") }
                    }
                    Button(onClick = { controller.saveConnection(serverUrl, token) }) {
                        Text("Connect")
                    }
                }
                state.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        }
    }
}

@Composable
private fun SessionList(
    state: TauUiState,
    controller: TauController,
    modifier: Modifier,
) {
    Column(modifier) {
        Row(
            Modifier.fillMaxWidth().padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text("Tau", style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.Bold)
                Text(
                    when (state.connectionStatus) {
                        ConnectionStatus.NotConfigured -> "Not configured"
                        ConnectionStatus.Connecting -> "Connecting"
                        ConnectionStatus.Connected -> "Connected"
                        ConnectionStatus.Offline -> "Offline"
                    },
                    color = when (state.connectionStatus) {
                        ConnectionStatus.Connected -> MaterialTheme.colorScheme.primary
                        ConnectionStatus.Offline -> MaterialTheme.colorScheme.error
                        else -> MaterialTheme.colorScheme.onSurfaceVariant
                    },
                    style = MaterialTheme.typography.labelMedium,
                )
            }
            TextButton(onClick = controller::showSettings) { Text("Settings") }
        }
        Button(
            onClick = controller::createSession,
            enabled = state.connectionStatus == ConnectionStatus.Connected,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
        ) {
            Text("New chat")
        }
        Spacer(Modifier.height(12.dp))
        HorizontalDivider()
        if (state.sessions.isEmpty()) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text(
                    if (state.connectionStatus == ConnectionStatus.Connected) {
                        "Create your first Tau chat."
                    } else {
                        "Waiting for the daemon."
                    },
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        } else {
            LazyColumn(
                contentPadding = PaddingValues(8.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                items(state.sessions, key = SessionSummary::id) { session ->
                    val selected = session.id == state.selectedSessionId
                    Card(
                        colors = CardDefaults.cardColors(
                            containerColor = if (selected) {
                                MaterialTheme.colorScheme.secondaryContainer
                            } else {
                                MaterialTheme.colorScheme.surface
                            },
                        ),
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable { controller.selectSession(session.id) },
                    ) {
                        Column(Modifier.padding(12.dp)) {
                            Text(
                                session.title,
                                maxLines = 2,
                                overflow = TextOverflow.Ellipsis,
                                fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
                            )
                            Spacer(Modifier.height(4.dp))
                            Text(
                                session.detail ?: session.status.label,
                                style = MaterialTheme.typography.labelSmall,
                                color = session.status.color,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ChatPanel(
    state: TauUiState,
    controller: TauController,
    showBack: Boolean,
    modifier: Modifier,
) {
    val sessionId = state.selectedSessionId
    val session = state.sessions.firstOrNull { it.id == sessionId }
    if (sessionId == null || session == null) {
        Box(modifier, contentAlignment = Alignment.Center) {
            Text("Select or create a chat.", color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        return
    }

    var renaming by remember(sessionId) { mutableStateOf(false) }
    var renameText by remember(session.title, renaming) { mutableStateOf(session.title) }
    val messages = state.histories[sessionId].orEmpty()
    val partial = state.partials[sessionId].orEmpty()
    val listState = rememberLazyListState()
    LaunchedEffect(messages.size, partial.length) {
        val count = messages.size + if (partial.isNotEmpty()) 1 else 0
        if (count > 0) listState.animateScrollToItem(count - 1)
    }

    Column(modifier) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (showBack) {
                TextButton(onClick = controller::showSessionList) { Text("Chats") }
            }
            Column(Modifier.weight(1f)) {
                Text(
                    session.title,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    session.detail ?: session.status.label,
                    style = MaterialTheme.typography.labelSmall,
                    color = session.status.color,
                )
            }
            if (session.status == SessionStatus.Starting) {
                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
            }
        }
        Row(
            Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            TextButton(onClick = { renaming = true }) { Text("Rename") }
            TextButton(onClick = controller::cloneSession) { Text("Clone") }
            TextButton(onClick = controller::closeSession) { Text("Sleep") }
        }
        HorizontalDivider()
        LazyColumn(
            state = listState,
            modifier = Modifier.weight(1f).fillMaxWidth(),
            contentPadding = PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (messages.isEmpty() && partial.isEmpty()) {
                item {
                    Text(
                        "Start a conversation with Pi.",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            items(messages, key = ChatMessage::entryId) { message ->
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = if (message.role == ChatRole.User) {
                        Arrangement.End
                    } else {
                        Arrangement.Start
                    },
                ) {
                    Card(
                        colors = CardDefaults.cardColors(
                            containerColor = when (message.role) {
                                ChatRole.User -> MaterialTheme.colorScheme.primaryContainer
                                ChatRole.Assistant -> MaterialTheme.colorScheme.surfaceVariant
                                ChatRole.System -> MaterialTheme.colorScheme.tertiaryContainer
                            },
                        ),
                        modifier = Modifier.fillMaxWidth(0.9f),
                    ) {
                        Column(Modifier.padding(12.dp)) {
                            SelectionContainer {
                                Text(message.text, style = MaterialTheme.typography.bodyLarge)
                            }
                            message.attachment?.let { attachment ->
                                OutlinedButton(onClick = { controller.downloadAttachment(message) }) {
                                    Text("Download ${attachment.fileName}")
                                }
                            }
                            if (message.role == ChatRole.User) {
                                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                                    TextButton(onClick = { controller.fork(message.entryId) }) {
                                        Text("Fork here")
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if (partial.isNotEmpty()) {
                item("streaming") {
                    Card(
                        colors = CardDefaults.cardColors(
                            containerColor = MaterialTheme.colorScheme.surfaceVariant,
                        ),
                        modifier = Modifier.fillMaxWidth(0.9f),
                    ) {
                        SelectionContainer {
                            Text(partial, Modifier.padding(12.dp))
                        }
                    }
                }
            }
        }
        HorizontalDivider()
        Row(
            Modifier.fillMaxWidth().padding(12.dp),
            verticalAlignment = Alignment.Bottom,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            OutlinedTextField(
                value = state.drafts[sessionId].orEmpty(),
                onValueChange = { controller.setDraft(sessionId, it) },
                placeholder = {
                    Text(if (session.status == SessionStatus.Running) "Steer Pi" else "Message Pi")
                },
                minLines = 1,
                maxLines = 8,
                modifier = Modifier.weight(1f),
            )
            if (session.status == SessionStatus.Running) {
                OutlinedButton(onClick = controller::abort) { Text("Abort") }
            }
            Button(
                onClick = controller::sendPrompt,
                enabled = state.connectionStatus == ConnectionStatus.Connected &&
                    state.drafts[sessionId].orEmpty().isNotBlank(),
            ) {
                Text(if (session.status == SessionStatus.Running) "Steer" else "Send")
            }
        }
    }

    if (renaming) {
        AlertDialog(
            onDismissRequest = { renaming = false },
            title = { Text("Rename chat") },
            text = {
                OutlinedTextField(
                    value = renameText,
                    onValueChange = { renameText = it },
                    singleLine = true,
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        controller.renameSession(renameText)
                        renaming = false
                    },
                    enabled = renameText.isNotBlank(),
                ) { Text("Save") }
            },
            dismissButton = {
                TextButton(onClick = { renaming = false }) { Text("Cancel") }
            },
        )
    }
}

private val SessionStatus.label: String
    get() = when (this) {
        SessionStatus.Sleeping -> "Sleeping"
        SessionStatus.Starting -> "Starting"
        SessionStatus.Idle -> "Idle"
        SessionStatus.Running -> "Running"
        SessionStatus.Error -> "Error"
    }

private val SessionStatus.color: Color
    @Composable get() = when (this) {
        SessionStatus.Idle -> MaterialTheme.colorScheme.primary
        SessionStatus.Running, SessionStatus.Starting -> MaterialTheme.colorScheme.tertiary
        SessionStatus.Error -> MaterialTheme.colorScheme.error
        SessionStatus.Sleeping -> MaterialTheme.colorScheme.onSurfaceVariant
    }
