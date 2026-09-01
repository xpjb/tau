@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)

package app.tau

import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.displayCutoutPadding
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
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
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.collectAsState
import kotlin.math.roundToInt
import kotlinx.coroutines.launch

private data class ChatListStructure(
    val messageCount: Int,
    val firstEntryId: String?,
    val lastEntryId: String?,
    val hasPartial: Boolean,
)

private val ConnectedColor = Color(0xFF4ADE80)
private val ReconnectingColor = Color(0xFFFBBF24)

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

    val selectedRunning = !state.editingSettings && state.sessions.any { session ->
        session.id == state.selectedSessionId && session.status == SessionStatus.Running
    }
    val chatStateHolder = rememberSaveableStateHolder()
    val chatIds = state.sessions.mapTo(mutableSetOf(), SessionSummary::id)
    var retainedChatIds by remember { mutableStateOf(emptySet<String>()) }
    LaunchedEffect(chatIds) {
        (retainedChatIds - chatIds).forEach(chatStateHolder::removeState)
        retainedChatIds = chatIds
    }
    MaterialTheme(colorScheme = TauDarkColors) {
        Surface(
            Modifier
                .fillMaxSize()
                .onInterruptShortcut(selectedRunning, controller::abort),
        ) {
            if (state.editingSettings) {
                ConnectionScreen(state, controller)
            } else {
                Box(
                    Modifier
                        .fillMaxSize()
                        .systemBarsPadding()
                        .displayCutoutPadding()
                        .imePadding(),
                ) {
                    BoxWithConstraints(Modifier.fillMaxSize()) {
                        val selectedSessionId = state.selectedSessionId
                        if (maxWidth >= 760.dp) {
                            Row(Modifier.fillMaxSize()) {
                                SessionList(
                                    state = state,
                                    controller = controller,
                                    modifier = Modifier.width(300.dp).fillMaxHeight(),
                                )
                                VerticalDivider()
                                if (selectedSessionId == null) {
                                    ChatPanel(
                                        state = state,
                                        controller = controller,
                                        showBack = false,
                                        modifier = Modifier.weight(1f).fillMaxHeight(),
                                    )
                                } else {
                                    chatStateHolder.SaveableStateProvider(selectedSessionId) {
                                        ChatPanel(
                                            state = state,
                                            controller = controller,
                                            showBack = false,
                                            modifier = Modifier.weight(1f).fillMaxHeight(),
                                        )
                                    }
                                }
                            }
                        } else if (state.mobileChatVisible && selectedSessionId != null) {
                            PlatformBackHandler(true, controller::showSessionList)
                            chatStateHolder.SaveableStateProvider(selectedSessionId) {
                                ChatPanel(
                                    state = state,
                                    controller = controller,
                                    showBack = true,
                                    modifier = Modifier.fillMaxSize(),
                                )
                            }
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
        Modifier
            .fillMaxSize()
            .systemBarsPadding()
            .displayCutoutPadding()
            .imePadding(),
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
private fun PositionedDropdownMenu(
    expanded: Boolean,
    pointerPosition: Offset?,
    onDismissRequest: () -> Unit,
    content: @Composable ColumnScope.() -> Unit,
) {
    if (pointerPosition == null) {
        DropdownMenu(
            expanded = expanded,
            onDismissRequest = onDismissRequest,
            content = content,
        )
        return
    }
    Box(
        Modifier
            .offset {
                IntOffset(
                    pointerPosition.x.roundToInt(),
                    pointerPosition.y.roundToInt(),
                )
            }
            .size(1.dp),
    ) {
        DropdownMenu(
            expanded = expanded,
            onDismissRequest = onDismissRequest,
            content = content,
        )
    }
}

@Composable
private fun ConnectionDot(status: ConnectionStatus) {
    val connected = status == ConnectionStatus.Connected
    Box(
        Modifier
            .size(9.dp)
            .background(
                color = if (connected) ConnectedColor else ReconnectingColor,
                shape = CircleShape,
            )
            .semantics {
                contentDescription = if (connected) "Connected" else "Reconnecting"
            },
    )
}

@Composable
private fun SessionList(
    state: TauUiState,
    controller: TauController,
    modifier: Modifier,
) {
    var renaming by remember { mutableStateOf<SessionSummary?>(null) }
    var deleting by remember { mutableStateOf<SessionSummary?>(null) }
    var renameText by remember { mutableStateOf("") }
    val actionsEnabled = state.connectionStatus == ConnectionStatus.Connected

    Column(modifier) {
        Row(
            Modifier.fillMaxWidth().padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Row(
                Modifier.weight(1f),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("Tau", style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.Bold)
                ConnectionDot(state.connectionStatus)
            }
            TextButton(onClick = controller::showSettings) { Text("Settings") }
        }
        Button(
            onClick = controller::createSession,
            enabled = actionsEnabled,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
        ) {
            Text("New chat")
        }
        Spacer(Modifier.height(12.dp))
        HorizontalDivider()
        if (state.sessions.isEmpty()) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text(
                    if (actionsEnabled) {
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
                    var menuExpanded by remember(session.id) { mutableStateOf(false) }
                    var menuPointer by remember(session.id) { mutableStateOf<Offset?>(null) }
                    Box(Modifier.fillMaxWidth()) {
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
                                .onSecondaryClick { position ->
                                    menuPointer = position
                                    menuExpanded = true
                                }
                                .combinedClickable(
                                    onClick = { controller.selectSession(session.id) },
                                    onLongClick = {
                                        menuPointer = null
                                        menuExpanded = true
                                    },
                                ),
                        ) {
                            Column(Modifier.padding(12.dp)) {
                                Text(
                                    session.title,
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                    fontWeight = if (selected) {
                                        FontWeight.SemiBold
                                    } else {
                                        FontWeight.Normal
                                    },
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
                        PositionedDropdownMenu(
                            expanded = menuExpanded,
                            pointerPosition = menuPointer,
                            onDismissRequest = {
                                menuExpanded = false
                                menuPointer = null
                            },
                        ) {
                            DropdownMenuItem(
                                text = { Text("Rename") },
                                enabled = actionsEnabled,
                                onClick = {
                                    menuExpanded = false
                                    menuPointer = null
                                    renameText = session.title
                                    renaming = session
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("Delete") },
                                enabled = actionsEnabled,
                                onClick = {
                                    menuExpanded = false
                                    menuPointer = null
                                    deleting = session
                                },
                            )
                        }
                    }
                }
            }
        }
    }

    renaming?.let { session ->
        AlertDialog(
            onDismissRequest = { renaming = null },
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
                        controller.renameSession(session.id, renameText)
                        renaming = null
                    },
                    enabled = actionsEnabled && renameText.isNotBlank(),
                ) { Text("Save") }
            },
            dismissButton = {
                TextButton(onClick = { renaming = null }) { Text("Cancel") }
            },
        )
    }

    deleting?.let { session ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            title = { Text("Delete chat?") },
            text = {
                Text("Permanently delete “${session.title}” and its Pi session history? This cannot be undone.")
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        controller.deleteSession(session.id)
                        deleting = null
                    },
                    enabled = actionsEnabled,
                ) { Text("Delete", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { deleting = null }) { Text("Cancel") }
            },
        )
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

    val messages = state.histories[sessionId].orEmpty()
    val partial = state.partials[sessionId].orEmpty()
    val attachments = state.attachments[sessionId].orEmpty()
    val draft = state.drafts[sessionId].orEmpty()
    val uploading = sessionId in state.uploadingSessions
    val canAttach = state.connectionStatus == ConnectionStatus.Connected &&
        !state.pickingFiles && !uploading
    val canSend = state.connectionStatus == ConnectionStatus.Connected &&
        !uploading &&
        (draft.isNotBlank() || attachments.isNotEmpty())
    var draggingFiles by remember(sessionId) { mutableStateOf(false) }
    var editorValue by remember(sessionId) {
        mutableStateOf(TextFieldValue(draft, TextRange(draft.length)))
    }
    var followBottom by rememberSaveable(sessionId) { mutableStateOf(true) }
    var originEntryId by rememberSaveable(sessionId) { mutableStateOf<String?>(null) }
    var originOffset by rememberSaveable(sessionId) { mutableStateOf(0) }
    var scrollingToBottom by remember { mutableStateOf(false) }
    val listState = rememberLazyListState()
    val scrollScope = rememberCoroutineScope()
    val listStructure = ChatListStructure(
        messageCount = messages.size,
        firstEntryId = messages.firstOrNull()?.entryId,
        lastEntryId = messages.lastOrNull()?.entryId,
        hasPartial = partial.isNotEmpty(),
    )
    var appliedListStructure by remember { mutableStateOf<ChatListStructure?>(null) }
    val showScrollToBottom by remember(listState) {
        derivedStateOf {
            listState.layoutInfo.totalItemsCount > 0 &&
                listState.layoutInfo.visibleItemsInfo.none { it.index == 0 }
        }
    }
    if (appliedListStructure != listStructure) {
        SideEffect {
            if (followBottom) {
                listState.requestScrollToItem(0)
            } else {
                val chronologicalIndex = messages.indexOfFirst { it.entryId == originEntryId }
                if (chronologicalIndex >= 0) {
                    val partialOffset = if (partial.isNotEmpty()) 1 else 0
                    val index = messages.lastIndex - chronologicalIndex + partialOffset
                    listState.requestScrollToItem(index, originOffset)
                }
            }
            appliedListStructure = listStructure
        }
    }
    LaunchedEffect(listState, listStructure) {
        snapshotFlow {
            Triple(
                listState.isScrollInProgress,
                listState.firstVisibleItemIndex,
                listState.firstVisibleItemScrollOffset,
            )
        }.collect { (scrolling, index, offset) ->
            if (appliedListStructure != listStructure || scrollingToBottom) return@collect
            val atBottom = index == 0 && offset == 0
            val follows = if (scrolling) atBottom else followBottom
            if (scrolling) followBottom = follows
            if (follows || atBottom) {
                if (atBottom) {
                    followBottom = true
                    originEntryId = null
                    originOffset = 0
                }
                return@collect
            }
            val partialOffset = if (partial.isNotEmpty()) 1 else 0
            val reverseIndex = index - partialOffset
            val chronologicalIndex = messages.lastIndex - reverseIndex
            messages.getOrNull(chronologicalIndex)?.let { message ->
                originEntryId = message.entryId
                originOffset = offset
            }
        }
    }
    LaunchedEffect(draft) {
        if (editorValue.text != draft) {
            editorValue = TextFieldValue(draft, TextRange(draft.length))
        }
    }

    Box(
        modifier.onFilesDropped(
            enabled = canAttach,
            onDraggingChanged = { draggingFiles = it },
            onDrop = controller::attachDroppedFiles,
        ),
    ) {
        Column(Modifier.fillMaxSize()) {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (showBack) {
                    IconButton(onClick = controller::showSessionList) {
                        Icon(
                            imageVector = BackIcon,
                            contentDescription = "Back to chats",
                            modifier = Modifier.size(24.dp),
                        )
                    }
                }
                Column(Modifier.weight(1f)) {
                    Row(
                        Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(
                            session.title,
                            modifier = Modifier.weight(1f),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            fontWeight = FontWeight.SemiBold,
                        )
                        ConnectionDot(state.connectionStatus)
                    }
                    Text(
                        session.detail ?: session.status.label,
                        style = MaterialTheme.typography.labelSmall,
                        color = session.status.color,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                if (session.status == SessionStatus.Starting) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                } else if (session.status == SessionStatus.Running) {
                    FilledTonalIconButton(
                        onClick = controller::abort,
                        modifier = Modifier.size(40.dp),
                    ) {
                        Icon(
                            imageVector = StopIcon,
                            contentDescription = "Interrupt Pi",
                            modifier = Modifier.size(20.dp),
                        )
                    }
                }
            }
            HorizontalDivider()
            Box(Modifier.weight(1f).fillMaxWidth()) {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(16.dp),
                    reverseLayout = true,
                    verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.Bottom),
                ) {
                    if (messages.isEmpty() && partial.isEmpty()) {
                        item {
                            Text(
                                "Start a conversation with Pi.",
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
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
                    items(messages.asReversed(), key = ChatMessage::entryId) { message ->
                        var menuExpanded by remember(message.entryId) { mutableStateOf(false) }
                        var menuPointer by remember(message.entryId) { mutableStateOf<Offset?>(null) }
                        Row(
                            Modifier.fillMaxWidth(),
                            horizontalArrangement = if (message.role == ChatRole.User) {
                                Arrangement.End
                            } else {
                                Arrangement.Start
                            },
                        ) {
                            Box(Modifier.fillMaxWidth(0.9f)) {
                                val menuModifier = if (message.role == ChatRole.User) {
                                    Modifier
                                        .onSecondaryClick { position ->
                                            menuPointer = position
                                            menuExpanded = true
                                        }
                                        .combinedClickable(
                                            onClick = {},
                                            onLongClick = {
                                                menuPointer = null
                                                menuExpanded = true
                                            },
                                        )
                                } else {
                                    Modifier
                                }
                                Card(
                                    colors = CardDefaults.cardColors(
                                        containerColor = when (message.role) {
                                            ChatRole.User -> MaterialTheme.colorScheme.primaryContainer
                                            ChatRole.Assistant -> MaterialTheme.colorScheme.surfaceVariant
                                            ChatRole.System -> MaterialTheme.colorScheme.tertiaryContainer
                                        },
                                    ),
                                    modifier = Modifier.fillMaxWidth().then(menuModifier),
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
                                    }
                                }
                                if (message.role == ChatRole.User) {
                                    PositionedDropdownMenu(
                                        expanded = menuExpanded,
                                        pointerPosition = menuPointer,
                                        onDismissRequest = {
                                            menuExpanded = false
                                            menuPointer = null
                                        },
                                    ) {
                                        DropdownMenuItem(
                                            text = { Text("Fork here") },
                                            onClick = {
                                                menuExpanded = false
                                                menuPointer = null
                                                controller.fork(message.entryId)
                                            },
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
                if (showScrollToBottom) {
                    FilledTonalIconButton(
                        onClick = {
                            scrollScope.launch {
                                scrollingToBottom = true
                                try {
                                    listState.animateScrollToItem(0)
                                } finally {
                                    followBottom = true
                                    originEntryId = null
                                    originOffset = 0
                                    scrollingToBottom = false
                                }
                            }
                        },
                        enabled = !scrollingToBottom,
                        modifier = Modifier
                            .align(Alignment.BottomEnd)
                            .padding(12.dp)
                            .size(44.dp),
                    ) {
                        Icon(
                            imageVector = ScrollToBottomIcon,
                            contentDescription = "Scroll to newest message",
                            modifier = Modifier.size(22.dp),
                        )
                    }
                }
            }
            HorizontalDivider()
            Column(
                Modifier.fillMaxWidth().padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (attachments.isNotEmpty()) {
                    Row(
                        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        attachments.forEachIndexed { index, file ->
                            OutlinedButton(
                                onClick = { controller.removeAttachment(sessionId, index) },
                                enabled = !uploading,
                            ) {
                                Text("${file.name}  ×", maxLines = 1)
                            }
                        }
                    }
                }
                OutlinedTextField(
                    value = editorValue,
                    onValueChange = {
                        editorValue = it
                        controller.setDraft(sessionId, it.text)
                    },
                    placeholder = { Text("Message Pi") },
                    minLines = 1,
                    maxLines = 8,
                    leadingIcon = {
                        IconButton(
                            onClick = controller::pickFiles,
                            enabled = canAttach,
                        ) {
                            if (state.pickingFiles) {
                                CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                            } else {
                                Icon(
                                    imageVector = AttachFileIcon,
                                    contentDescription = "Attach files",
                                    modifier = Modifier.size(22.dp),
                                )
                            }
                        }
                    },
                    trailingIcon = {
                        if (uploading) {
                            Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
                                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                            }
                        } else {
                            FilledIconButton(
                                onClick = controller::sendPrompt,
                                enabled = canSend,
                                modifier = Modifier.size(40.dp),
                            ) {
                                Icon(
                                    imageVector = SendIcon,
                                    contentDescription = if (session.status == SessionStatus.Running) {
                                        "Steer Pi"
                                    } else {
                                        "Send message"
                                    },
                                    modifier = Modifier.size(20.dp),
                                )
                            }
                        }
                    },
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                    keyboardActions = KeyboardActions(
                        onSend = { if (canSend) controller.sendPrompt() },
                    ),
                    modifier = Modifier
                        .fillMaxWidth()
                        .onClipboardImagePaste(canAttach, controller::attachClipboardImage)
                        .onPreviewKeyEvent { event ->
                            if (event.key != Key.Enter) {
                                false
                            } else if (event.isShiftPressed) {
                                if (event.type == KeyEventType.KeyDown) {
                                    val start = minOf(editorValue.selection.start, editorValue.selection.end)
                                    val end = maxOf(editorValue.selection.start, editorValue.selection.end)
                                    val updated = editorValue.text.replaceRange(start, end, "\n")
                                    editorValue = TextFieldValue(updated, TextRange(start + 1))
                                    controller.setDraft(sessionId, updated)
                                }
                                true
                            } else {
                                if (event.type == KeyEventType.KeyDown && canSend) {
                                    controller.sendPrompt()
                                }
                                true
                            }
                        },
                )
            }
        }
        if (draggingFiles) {
            Surface(
                modifier = Modifier.fillMaxSize().padding(12.dp),
                color = MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.96f),
                shape = MaterialTheme.shapes.large,
                tonalElevation = 8.dp,
            ) {
                Column(
                    Modifier.fillMaxSize(),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    Icon(
                        imageVector = AttachFileIcon,
                        contentDescription = null,
                        modifier = Modifier.size(40.dp),
                    )
                    Spacer(Modifier.height(12.dp))
                    Text(
                        "Drop files to attach",
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.SemiBold,
                    )
                }
            }
        }
    }
}

private val BackIcon = ImageVector.Builder(
    name = "Back",
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 24f,
    viewportHeight = 24f,
).apply {
    path(fill = SolidColor(Color.Black)) {
        moveTo(20f, 11f)
        horizontalLineTo(7.83f)
        lineTo(13.42f, 5.41f)
        lineTo(12f, 4f)
        lineTo(4f, 12f)
        lineTo(12f, 20f)
        lineTo(13.42f, 18.59f)
        lineTo(7.83f, 13f)
        horizontalLineTo(20f)
        close()
    }
}.build()

private val ScrollToBottomIcon = ImageVector.Builder(
    name = "ScrollToBottom",
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 24f,
    viewportHeight = 24f,
).apply {
    path(fill = SolidColor(Color.Black)) {
        moveTo(7.41f, 8.59f)
        lineTo(12f, 13.17f)
        lineTo(16.59f, 8.59f)
        lineTo(18f, 10f)
        lineTo(12f, 16f)
        lineTo(6f, 10f)
        close()
    }
}.build()

private val AttachFileIcon = ImageVector.Builder(
    name = "AttachFile",
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 24f,
    viewportHeight = 24f,
).apply {
    path(fill = SolidColor(Color.Black)) {
        moveTo(16.5f, 6f)
        verticalLineTo(17.5f)
        curveTo(16.5f, 19.71f, 14.71f, 21.5f, 12.5f, 21.5f)
        curveTo(10.29f, 21.5f, 8.5f, 19.71f, 8.5f, 17.5f)
        verticalLineTo(5f)
        curveTo(8.5f, 3.62f, 9.62f, 2.5f, 11f, 2.5f)
        curveTo(12.38f, 2.5f, 13.5f, 3.62f, 13.5f, 5f)
        verticalLineTo(15.5f)
        curveTo(13.5f, 16.05f, 13.05f, 16.5f, 12.5f, 16.5f)
        curveTo(11.95f, 16.5f, 11.5f, 16.05f, 11.5f, 15.5f)
        verticalLineTo(6f)
        horizontalLineTo(10f)
        verticalLineTo(15.5f)
        curveTo(10f, 16.88f, 11.12f, 18f, 12.5f, 18f)
        curveTo(13.88f, 18f, 15f, 16.88f, 15f, 15.5f)
        verticalLineTo(5f)
        curveTo(15f, 2.79f, 13.21f, 1f, 11f, 1f)
        curveTo(8.79f, 1f, 7f, 2.79f, 7f, 5f)
        verticalLineTo(17.5f)
        curveTo(7f, 20.54f, 9.46f, 23f, 12.5f, 23f)
        curveTo(15.54f, 23f, 18f, 20.54f, 18f, 17.5f)
        verticalLineTo(6f)
        close()
    }
}.build()

private val SendIcon = ImageVector.Builder(
    name = "Send",
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 24f,
    viewportHeight = 24f,
).apply {
    path(fill = SolidColor(Color.Black)) {
        moveTo(2.01f, 21f)
        lineTo(23f, 12f)
        lineTo(2.01f, 3f)
        lineTo(2f, 10f)
        lineTo(17f, 12f)
        lineTo(2f, 14f)
        close()
    }
}.build()

private val StopIcon = ImageVector.Builder(
    name = "Stop",
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 24f,
    viewportHeight = 24f,
).apply {
    path(fill = SolidColor(Color.Black)) {
        moveTo(6f, 6f)
        horizontalLineTo(18f)
        verticalLineTo(18f)
        horizontalLineTo(6f)
        close()
    }
}.build()

private val SessionStatus.label: String
    get() = when (this) {
        SessionStatus.Sleeping -> "Ready"
        SessionStatus.Starting -> "Starting"
        SessionStatus.Idle -> "Ready"
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
