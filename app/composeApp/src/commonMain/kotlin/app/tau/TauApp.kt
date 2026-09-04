@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)

package app.tau

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.text.selection.SelectionState
import androidx.compose.foundation.text.selection.rememberSelectionState
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
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.LocalContentColor
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
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
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
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import coil3.ImageLoader
import coil3.compose.AsyncImage
import coil3.compose.LocalPlatformContext
import coil3.compose.setSingletonImageLoaderFactory
import coil3.disk.DiskCache
import coil3.network.NetworkHeaders
import coil3.network.httpHeaders
import coil3.network.ktor3.KtorNetworkFetcherFactory
import coil3.request.CachePolicy
import coil3.request.ImageRequest
import coil3.serviceLoaderEnabled
import io.ktor.client.HttpClient
import io.ktor.http.encodeURLPathPart
import kotlinx.coroutines.delay
import okio.Path.Companion.toPath
import kotlin.math.roundToInt

private val ConnectedColor = Color(0xFF4ADE80)
private val ReconnectingColor = Color(0xFFFBBF24)
private val InlineImagePreviewHeight = 260.dp
private val AttachmentTopSpacing = 8.dp
private val AttachmentControlHeight = 68.dp
private val ImageDownloadSpacing = 4.dp

private data class ComposerSuggestion(
    val value: String,
    val label: String,
    val description: String?,
    val replaceStart: Int,
    val replaceEnd: Int,
)

private fun formatByteCount(bytes: Long): String {
    val amount = bytes.coerceAtLeast(0)
    if (amount < 1_024) return "$amount B"
    val units = listOf("KB", "MB", "GB")
    var unit = 1_024L
    var index = 0
    while (index < units.lastIndex && amount >= unit * 1_024) {
        unit *= 1_024
        index++
    }
    val tenths = (amount * 10 + unit / 2) / unit
    return if (tenths < 100) {
        "${tenths / 10}.${tenths % 10} ${units[index]}"
    } else {
        "${(tenths + 5) / 10} ${units[index]}"
    }
}

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
    setSingletonImageLoaderFactory { context ->
        ImageLoader.Builder(context)
            .serviceLoaderEnabled(false)
            .diskCache {
                DiskCache.Builder()
                    .directory(PlatformServices.thumbnailCacheDirectory.toPath())
                    .maxSizeBytes(100L * 1_024 * 1_024)
                    .build()
            }
            .components {
                add(
                    KtorNetworkFetcherFactory(
                        httpClient = { HttpClient(platformHttpEngine()) },
                    ),
                )
            }
            .build()
    }
    val state by controller.state.collectAsState()
    DisposableEffect(controller) {
        controller.start()
        onDispose(controller::dispose)
    }

    val selectedRunning = !state.editingSettings && state.sessions.any { session ->
        session.id == state.selectedSessionId && session.status == SessionStatus.Running
    }
    val chatStateHolder = rememberSaveableStateHolder()
    val transcriptMeasureCaches = remember { mutableMapOf<String, TranscriptMeasureCache>() }
    val chatIds = state.sessions.mapTo(mutableSetOf(), SessionSummary::id)
    var retainedChatIds by remember { mutableStateOf(emptySet<String>()) }
    LaunchedEffect(chatIds) {
        (retainedChatIds - chatIds).forEach { sessionId ->
            chatStateHolder.removeState(sessionId)
            transcriptMeasureCaches.remove(sessionId)
        }
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
                                        transcriptMeasureCaches = transcriptMeasureCaches,
                                        modifier = Modifier.weight(1f).fillMaxHeight(),
                                    )
                                } else {
                                    chatStateHolder.SaveableStateProvider(selectedSessionId) {
                                        ChatPanel(
                                            state = state,
                                            controller = controller,
                                            showBack = false,
                                            transcriptMeasureCaches = transcriptMeasureCaches,
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
                                    transcriptMeasureCaches = transcriptMeasureCaches,
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
        state.extensionDialogs.firstOrNull()?.let { dialog ->
            val request = dialog.request
            var responseText by remember(dialog.sessionId, request.id) {
                mutableStateOf(request.prefill.orEmpty())
            }
            LaunchedEffect(dialog.sessionId, request.id, request.timeout) {
                request.timeout?.let { timeout ->
                    delay(timeout.coerceAtLeast(0))
                    controller.dismissExpiredExtensionUi(dialog)
                }
            }
            val dismiss = { controller.respondExtensionUi(dialog, cancelled = true) }
            when (request.method) {
                "select" -> AlertDialog(
                    onDismissRequest = dismiss,
                    title = { Text(request.title ?: "Choose an option") },
                    text = {
                        LazyColumn(Modifier.fillMaxWidth().heightIn(max = 360.dp)) {
                            items(request.options) { option ->
                                TextButton(
                                    onClick = {
                                        controller.respondExtensionUi(dialog, value = option)
                                    },
                                    modifier = Modifier.fillMaxWidth(),
                                ) {
                                    Text(option, modifier = Modifier.fillMaxWidth())
                                }
                            }
                        }
                    },
                    confirmButton = {},
                    dismissButton = {
                        TextButton(onClick = dismiss) { Text("Cancel") }
                    },
                )
                "confirm" -> AlertDialog(
                    onDismissRequest = dismiss,
                    title = { Text(request.title ?: "Confirm") },
                    text = { Text(request.message.orEmpty()) },
                    confirmButton = {
                        TextButton(
                            onClick = {
                                controller.respondExtensionUi(dialog, confirmed = true)
                            },
                        ) { Text("Yes") }
                    },
                    dismissButton = {
                        Row {
                            TextButton(
                                onClick = {
                                    controller.respondExtensionUi(dialog, confirmed = false)
                                },
                            ) { Text("No") }
                            TextButton(onClick = dismiss) { Text("Cancel") }
                        }
                    },
                )
                "input", "editor" -> AlertDialog(
                    onDismissRequest = dismiss,
                    title = {
                        Text(
                            request.title ?: if (request.method == "editor") {
                                "Edit text"
                            } else {
                                "Enter a value"
                            },
                        )
                    },
                    text = {
                        OutlinedTextField(
                            value = responseText,
                            onValueChange = { responseText = it },
                            placeholder = request.placeholder?.let { placeholder ->
                                { Text(placeholder) }
                            },
                            minLines = if (request.method == "editor") 4 else 1,
                            maxLines = if (request.method == "editor") 14 else 1,
                            singleLine = request.method == "input",
                            modifier = Modifier.fillMaxWidth(),
                        )
                    },
                    confirmButton = {
                        TextButton(
                            onClick = {
                                controller.respondExtensionUi(dialog, value = responseText)
                            },
                        ) { Text("Submit") }
                    },
                    dismissButton = {
                        TextButton(onClick = dismiss) { Text("Cancel") }
                    },
                )
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
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text("Tau", style = MaterialTheme.typography.headlineLarge, fontWeight = FontWeight.Bold)
                    Text(
                        "Version ${PlatformServices.appVersion}",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
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

private sealed interface TranscriptRow {
    val key: String

    data object BottomAnchor : TranscriptRow {
        override val key = "bottom-anchor"
    }

    data object Empty : TranscriptRow {
        override val key = "empty"
    }

    data class Outgoing(val message: OutgoingMessage) : TranscriptRow {
        override val key = "outgoing-${message.requestId}"
    }

    data class Partial(
        override val key: String,
        val text: String,
    ) : TranscriptRow

    data class Message(
        override val key: String,
        val message: ChatMessage,
        val timestamp: String?,
    ) : TranscriptRow
}

private data class TranscriptMeasureContext(
    val width: Int,
    val density: Float,
    val fontScale: Float,
    val layoutDirection: androidx.compose.ui.unit.LayoutDirection,
    val textMeasurer: TextMeasurer,
    val styles: TranscriptTextStyles,
)

private data class MeasuredTranscriptRow(
    val text: MeasuredTranscriptText?,
    val height: Int,
)

private class TranscriptMeasureCache {
    var styles: TranscriptTextStyles? = null
    var context: TranscriptMeasureContext? = null
    val documents = mutableMapOf<TranscriptRow, TranscriptTextDocument>()
    val rows = mutableMapOf<TranscriptRow, MeasuredTranscriptRow>()
}

@Composable
private fun TranscriptRowContent(
    row: TranscriptRow,
    measured: MeasuredTranscriptRow,
    styles: TranscriptTextStyles,
    controller: TauController,
    settings: ConnectionSettings,
    sessionId: String,
    attachmentDownload: AttachmentDownload?,
    selectionState: SelectionState,
) {
    when (row) {
        TranscriptRow.BottomAnchor -> Spacer(Modifier.height(1.dp))
        TranscriptRow.Empty -> Text(
            "Start a conversation with Pi.",
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            style = styles.body,
        )
        is TranscriptRow.Outgoing -> Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.End,
        ) {
            Card(
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.72f),
                ),
                modifier = Modifier.fillMaxWidth(0.9f),
            ) {
                Column(
                    Modifier.padding(12.dp),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    ChatText(checkNotNull(measured.text), styles)
                    DisableSelection {
                        Text(
                            "Waiting for Pi",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.72f),
                        )
                    }
                }
            }
        }
        is TranscriptRow.Partial -> Card(
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceVariant,
            ),
            modifier = Modifier.fillMaxWidth(0.9f),
        ) {
            ChatText(
                text = checkNotNull(measured.text),
                styles = styles,
                modifier = Modifier.padding(12.dp),
            )
        }
        is TranscriptRow.Message -> {
            val message = row.message
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
                    val menuModifier = Modifier
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
                        Column(Modifier.fillMaxWidth().padding(12.dp)) {
                            ChatText(checkNotNull(measured.text), styles)
                            message.attachment?.let { attachment ->
                                DisableSelection {
                                    Column(Modifier.fillMaxWidth()) {
                                        Spacer(Modifier.height(AttachmentTopSpacing))
                                        if (attachment.kind == AttachmentKind.Image) {
                                            val platformContext = LocalPlatformContext.current
                                            val imageUrl = remember(
                                                settings.serverUrl,
                                                sessionId,
                                                message.entryId,
                                            ) {
                                                val baseUrl = settings.serverUrl.trim().trimEnd('/')
                                                "$baseUrl/v1/sessions/" +
                                                    sessionId.encodeURLPathPart() +
                                                    "/attachments/" +
                                                    message.entryId.encodeURLPathPart()
                                            }
                                            val thumbnailUrl = "$imageUrl/thumbnail"
                                            val authenticationHeaders = remember(settings.token) {
                                                NetworkHeaders.Builder()
                                                    .set(
                                                        "Authorization",
                                                        "Bearer ${settings.token}",
                                                    )
                                                    .set("Accept", "image/*")
                                                    .build()
                                            }
                                            val cacheScope = settings.token.hashCode()
                                            val thumbnailCacheKey = "$thumbnailUrl#$cacheScope"
                                            val thumbnailRequest = remember(
                                                platformContext,
                                                thumbnailUrl,
                                                authenticationHeaders,
                                                thumbnailCacheKey,
                                            ) {
                                                ImageRequest.Builder(platformContext)
                                                    .data(thumbnailUrl)
                                                    .httpHeaders(authenticationHeaders)
                                                    .memoryCacheKey(thumbnailCacheKey)
                                                    .diskCacheKey(thumbnailCacheKey)
                                                    .diskCachePolicy(CachePolicy.ENABLED)
                                                    .build()
                                            }
                                            val originalImageRequest = remember(
                                                platformContext,
                                                imageUrl,
                                                authenticationHeaders,
                                                cacheScope,
                                                thumbnailCacheKey,
                                            ) {
                                                ImageRequest.Builder(platformContext)
                                                    .data(imageUrl)
                                                    .httpHeaders(authenticationHeaders)
                                                    .memoryCacheKey("$imageUrl#$cacheScope")
                                                    .placeholderMemoryCacheKey(thumbnailCacheKey)
                                                    .diskCachePolicy(CachePolicy.DISABLED)
                                                    .build()
                                            }
                                            var useOriginalPreview by remember(imageUrl, settings.token) {
                                                mutableStateOf(false)
                                            }
                                            var imageLoaded by remember(imageUrl, settings.token) {
                                                mutableStateOf<Boolean?>(null)
                                            }
                                            var imageExpanded by remember(imageUrl, settings.token) {
                                                mutableStateOf(false)
                                            }
                                            val previewRequest = if (useOriginalPreview) {
                                                originalImageRequest
                                            } else {
                                                thumbnailRequest
                                            }
                                            Box(
                                                Modifier
                                                    .widthIn(max = 520.dp)
                                                    .fillMaxWidth()
                                                    .height(InlineImagePreviewHeight)
                                                    .clip(RoundedCornerShape(8.dp))
                                                    .background(
                                                        MaterialTheme.colorScheme.surface.copy(
                                                            alpha = 0.52f,
                                                        ),
                                                    )
                                                    .clickable { imageExpanded = true },
                                                contentAlignment = Alignment.Center,
                                            ) {
                                                when (imageLoaded) {
                                                    null -> CircularProgressIndicator(
                                                        Modifier.size(28.dp),
                                                        strokeWidth = 2.dp,
                                                    )
                                                    false -> Text(
                                                        "Image preview unavailable",
                                                        modifier = Modifier.padding(12.dp),
                                                        style = MaterialTheme.typography.labelMedium,
                                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                                    )
                                                    true -> Unit
                                                }
                                                AsyncImage(
                                                    model = previewRequest,
                                                    contentDescription = attachment.caption
                                                        ?: attachment.fileName,
                                                    modifier = Modifier.fillMaxSize(),
                                                    onLoading = { imageLoaded = null },
                                                    onSuccess = { imageLoaded = true },
                                                    onError = {
                                                        if (useOriginalPreview) {
                                                            imageLoaded = false
                                                        } else {
                                                            useOriginalPreview = true
                                                        }
                                                    },
                                                    contentScale = ContentScale.Fit,
                                                )
                                            }
                                            if (imageExpanded) {
                                                Dialog(
                                                    onDismissRequest = { imageExpanded = false },
                                                    properties = DialogProperties(
                                                        usePlatformDefaultWidth = false,
                                                    ),
                                                ) {
                                                    Box(
                                                        Modifier
                                                            .fillMaxSize()
                                                            .background(Color.Black.copy(alpha = 0.94f))
                                                            .clickable { imageExpanded = false }
                                                            .systemBarsPadding()
                                                            .displayCutoutPadding(),
                                                    ) {
                                                        AsyncImage(
                                                            model = originalImageRequest,
                                                            contentDescription = attachment.caption
                                                                ?: attachment.fileName,
                                                            modifier = Modifier
                                                                .fillMaxSize()
                                                                .padding(24.dp),
                                                            contentScale = ContentScale.Fit,
                                                        )
                                                        FilledTonalIconButton(
                                                            onClick = { imageExpanded = false },
                                                            modifier = Modifier
                                                                .align(Alignment.TopEnd)
                                                                .padding(8.dp),
                                                        ) {
                                                            Icon(
                                                                imageVector = CloseIcon,
                                                                contentDescription = "Close image",
                                                            )
                                                        }
                                                    }
                                                }
                                            }
                                            Spacer(Modifier.height(ImageDownloadSpacing))
                                        }
                                        val totalBytes = attachmentDownload?.totalBytes
                                            ?: attachment.size
                                        val statusText = when (attachmentDownload?.status) {
                                            AttachmentDownloadStatus.Downloading -> buildString {
                                                append(formatByteCount(attachmentDownload.transferredBytes))
                                                totalBytes?.takeIf { it > 0 }?.let { total ->
                                                    append(" / ").append(formatByteCount(total))
                                                    append(" · ")
                                                    append(
                                                        (attachmentDownload.transferredBytes * 100 / total)
                                                            .coerceIn(0, 100),
                                                    )
                                                    append('%')
                                                }
                                                attachmentDownload.bytesPerSecond
                                                    ?.takeIf { it > 0 }
                                                    ?.let { rate ->
                                                        append(" · ").append(formatByteCount(rate)).append("/s")
                                                    }
                                            }
                                            AttachmentDownloadStatus.Downloaded -> totalBytes
                                                ?.let { "${formatByteCount(it)} · Downloaded" }
                                                ?: "Downloaded"
                                            AttachmentDownloadStatus.Failed -> buildString {
                                                append(formatByteCount(attachmentDownload.transferredBytes))
                                                totalBytes?.let {
                                                    append(" / ").append(formatByteCount(it))
                                                }
                                                append(" · ")
                                                append(attachmentDownload.error ?: "Download failed")
                                            }
                                            null -> totalBytes?.let(::formatByteCount) ?: "Ready to download"
                                        }
                                        val progress = if (
                                            attachmentDownload?.status ==
                                            AttachmentDownloadStatus.Downloading &&
                                            totalBytes != null && totalBytes > 0
                                        ) {
                                            (attachmentDownload.transferredBytes.toFloat() / totalBytes)
                                                .coerceIn(0f, 1f)
                                        } else {
                                            null
                                        }
                                        Surface(
                                            color = MaterialTheme.colorScheme.surface.copy(alpha = 0.52f),
                                            shape = MaterialTheme.shapes.medium,
                                            modifier = Modifier
                                                .fillMaxWidth()
                                                .height(AttachmentControlHeight),
                                        ) {
                                            Column(
                                                Modifier.fillMaxSize().padding(
                                                    start = 12.dp,
                                                    top = 4.dp,
                                                    end = 4.dp,
                                                    bottom = 4.dp,
                                                ),
                                            ) {
                                                Row(
                                                    Modifier.weight(1f),
                                                    verticalAlignment = Alignment.CenterVertically,
                                                ) {
                                                    Column(Modifier.weight(1f)) {
                                                        Text(
                                                            attachment.fileName,
                                                            maxLines = 1,
                                                            overflow = TextOverflow.Ellipsis,
                                                            style = MaterialTheme.typography.bodyMedium,
                                                            fontWeight = FontWeight.Medium,
                                                        )
                                                        Text(
                                                            statusText,
                                                            maxLines = 1,
                                                            overflow = TextOverflow.Ellipsis,
                                                            style = MaterialTheme.typography.labelSmall,
                                                            color = if (
                                                                attachmentDownload?.status ==
                                                                AttachmentDownloadStatus.Failed
                                                            ) {
                                                                MaterialTheme.colorScheme.error
                                                            } else {
                                                                MaterialTheme.colorScheme.onSurfaceVariant
                                                            },
                                                        )
                                                    }
                                                    when (attachmentDownload?.status) {
                                                        AttachmentDownloadStatus.Downloading -> TextButton(
                                                            onClick = {
                                                                controller.cancelAttachmentDownload(message)
                                                            },
                                                        ) { Text("Cancel") }
                                                        AttachmentDownloadStatus.Downloaded -> TextButton(
                                                            onClick = {
                                                                controller.openAttachmentDownload(message)
                                                            },
                                                        ) { Text("Open") }
                                                        AttachmentDownloadStatus.Failed -> TextButton(
                                                            onClick = {
                                                                controller.downloadAttachment(message)
                                                            },
                                                        ) { Text("Retry") }
                                                        null -> TextButton(
                                                            onClick = {
                                                                controller.downloadAttachment(message)
                                                            },
                                                        ) { Text("Download") }
                                                    }
                                                }
                                                if (
                                                    attachmentDownload?.status ==
                                                    AttachmentDownloadStatus.Downloading
                                                ) {
                                                    if (progress == null) {
                                                        LinearProgressIndicator(Modifier.fillMaxWidth())
                                                    } else {
                                                        LinearProgressIndicator(
                                                            progress = { progress },
                                                            modifier = Modifier.fillMaxWidth(),
                                                        )
                                                    }
                                                } else {
                                                    Spacer(Modifier.height(4.dp))
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            row.timestamp?.let { timestamp ->
                                DisableSelection {
                                    Text(
                                        timestamp,
                                        modifier = Modifier.align(Alignment.End),
                                        style = MaterialTheme.typography.labelSmall,
                                        color = LocalContentColor.current.copy(alpha = 0.62f),
                                    )
                                }
                            }
                        }
                    }
                    DisableSelection {
                        PositionedDropdownMenu(
                            expanded = menuExpanded,
                            pointerPosition = menuPointer,
                            onDismissRequest = {
                                menuExpanded = false
                                menuPointer = null
                            },
                        ) {
                            val hasSelection = selectionState.selectedTexts.any { it.text.isNotEmpty() }
                            DropdownMenuItem(
                                text = {
                                    Text(if (hasSelection) "Copy selection" else "Copy message")
                                },
                                onClick = {
                                    val selectedText = selectionState.selectedTexts
                                        .joinToString(separator = "\n") { it.text }
                                    PlatformServices.copyText(
                                        selectedText.ifEmpty { message.text },
                                    )
                                    menuExpanded = false
                                    menuPointer = null
                                },
                            )
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
    val focusManager = LocalFocusManager.current
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
            onClick = {
                focusManager.clearFocus(force = true)
                controller.createSession()
            },
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
                items(state.sessions) { session ->
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
    transcriptMeasureCaches: MutableMap<String, TranscriptMeasureCache>,
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

    val messages = state.histories[sessionId]
    if (messages == null) {
        Box(modifier, contentAlignment = Alignment.Center) {
            CircularProgressIndicator(Modifier.size(28.dp), strokeWidth = 2.dp)
        }
        return
    }
    val outgoingMessages = state.outgoingMessages[sessionId].orEmpty()
    val partial = state.partials[sessionId].orEmpty()
    val attachments = state.attachments[sessionId].orEmpty()
    val draft = state.drafts[sessionId].orEmpty()
    val editorFocusRequester = remember(sessionId) { FocusRequester() }
    val keyboardController = LocalSoftwareKeyboardController.current
    val shouldFocusComposer = state.focusComposerSessionId == sessionId
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
    val availableCommands = state.slashCommands[sessionId].orEmpty()
    val suggestions = if (
        editorValue.selection.collapsed &&
        editorValue.selection.end == editorValue.text.length &&
        editorValue.text.startsWith('/')
    ) {
        val rest = editorValue.text.drop(1)
        val separator = rest.indexOfFirst { character -> character.isWhitespace() }
        if (separator < 0) {
            val prefix = rest
            availableCommands
                .asSequence()
                .filter { command -> command.name.contains(prefix, ignoreCase = true) }
                .sortedWith(
                    compareBy<SlashCommand> {
                        !it.name.startsWith(prefix, ignoreCase = true)
                    }.thenBy(SlashCommand::name),
                )
                .take(6)
                .map { command ->
                    ComposerSuggestion(
                        value = "/${command.name}",
                        label = buildString {
                            append('/').append(command.name)
                            command.argumentHint?.let { append(' ').append(it) }
                        },
                        description = command.description,
                        replaceStart = 0,
                        replaceEnd = editorValue.text.length,
                    )
                }
                .toList()
        } else {
            val commandName = rest.take(separator)
            val argumentStart = (separator + 1)
                .let { start ->
                    var index = start
                    while (index < rest.length && rest[index].isWhitespace()) index++
                    index + 1
                }
            val argumentPrefix = editorValue.text.substring(argumentStart)
            availableCommands
                .firstOrNull { command -> command.name == commandName }
                ?.arguments
                .orEmpty()
                .asSequence()
                .filter { argument ->
                    argumentPrefix.none { character -> character.isWhitespace() } &&
                        argument.value.contains(argumentPrefix, ignoreCase = true)
                }
                .sortedWith(
                    compareBy<SlashCommandArgument> {
                        !it.value.startsWith(argumentPrefix, ignoreCase = true)
                    }.thenBy(SlashCommandArgument::value),
                )
                .take(6)
                .map { argument ->
                    ComposerSuggestion(
                        value = argument.value,
                        label = argument.value,
                        description = argument.description,
                        replaceStart = argumentStart,
                        replaceEnd = editorValue.text.length,
                    )
                }
                .toList()
        }
    } else {
        emptyList()
    }
    var selectedSuggestion by remember(sessionId) { mutableStateOf(0) }
    val suggestionIdentity = suggestions.joinToString("\u0000") { suggestion -> suggestion.value }
    LaunchedEffect(suggestionIdentity) {
        selectedSuggestion = 0
    }
    val applySuggestion: (ComposerSuggestion) -> Unit = { suggestion ->
        val completed = editorValue.text.replaceRange(
            suggestion.replaceStart,
            suggestion.replaceEnd,
            "${suggestion.value} ",
        )
        editorValue = TextFieldValue(completed, TextRange(completed.length))
        controller.setDraft(sessionId, completed)
    }
    val listState = rememberLazyListState()
    val transcriptSelectionState = rememberSelectionState()
    val density = LocalDensity.current
    val layoutDirection = LocalLayoutDirection.current
    val textMeasurer = rememberTextMeasurer(cacheSize = 0)
    val transcriptTextStyles = TranscriptTextStyles(
        body = MaterialTheme.typography.bodyLarge,
        headings = listOf(
            MaterialTheme.typography.headlineMedium,
            MaterialTheme.typography.headlineSmall,
            MaterialTheme.typography.titleLarge,
            MaterialTheme.typography.titleMedium,
            MaterialTheme.typography.titleSmall,
            MaterialTheme.typography.labelLarge,
        ),
        code = MaterialTheme.typography.bodyLarge.copy(fontFamily = FontFamily.Monospace),
        link = SpanStyle(
            color = MaterialTheme.colorScheme.primary,
            fontWeight = FontWeight.Bold,
            textDecoration = TextDecoration.Underline,
        ),
        inlineCode = MaterialTheme.typography.bodyLarge.toSpanStyle().copy(
            fontFamily = FontFamily.Monospace,
            background = MaterialTheme.colorScheme.outlineVariant,
        ),
        codeBackground = MaterialTheme.colorScheme.background.copy(alpha = 0.72f),
        quoteBar = MaterialTheme.colorScheme.primary.copy(alpha = 0.72f),
    )
    val transcriptRows = remember(messages, outgoingMessages, partial) {
        buildList {
            add(TranscriptRow.BottomAnchor)
            if (messages.isEmpty() && outgoingMessages.isEmpty() && partial.isEmpty()) {
                add(TranscriptRow.Empty)
            }
            outgoingMessages.asReversed().forEach { outgoing ->
                add(TranscriptRow.Outgoing(outgoing))
            }
            if (partial.isNotEmpty()) {
                add(
                    TranscriptRow.Partial(
                        key = "assistant-after-${messages.lastOrNull()?.entryId ?: "start"}",
                        text = partial,
                    ),
                )
            }
            messages.indices.reversed().forEach { index ->
                val message = messages[index]
                val key = if (message.role == ChatRole.Assistant) {
                    "assistant-after-${messages.getOrNull(index - 1)?.entryId ?: "start"}"
                } else {
                    "message-${message.entryId}"
                }
                add(
                    TranscriptRow.Message(
                        key = key,
                        message = message,
                        timestamp = message.timestampMs
                            ?.let(PlatformServices::formatMessageTime)
                            ?.takeIf(String::isNotEmpty),
                    ),
                )
            }
        }
    }
    val transcriptMeasureCache = transcriptMeasureCaches.getOrPut(sessionId, ::TranscriptMeasureCache)
    val scrollMotion = rememberTranscriptScrollMotion()
    val showScrollToBottom by remember(listState) {
        derivedStateOf {
            listState.firstVisibleItemIndex != 0 || listState.firstVisibleItemScrollOffset != 0
        }
    }
    LaunchedEffect(sessionId) {
        transcriptSelectionState.clear()
    }
    LaunchedEffect(draft) {
        if (editorValue.text != draft) {
            editorValue = TextFieldValue(draft, TextRange(draft.length))
        }
    }
    LaunchedEffect(shouldFocusComposer) {
        if (shouldFocusComposer) {
            editorFocusRequester.requestFocus()
            keyboardController?.show()
            controller.consumeComposerFocus(sessionId)
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
                        session.detail
                            ?: state.extensionStatuses[sessionId]
                                .orEmpty()
                                .toSortedMap()
                                .values
                                .joinToString(" · ")
                                .ifEmpty { session.status.label },
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
            BoxWithConstraints(
                Modifier
                    .weight(1f)
                    .fillMaxWidth()
                    .onTranscriptAutoscroll(listState),
            ) {
                val transcriptPadding = with(density) { 16.dp.roundToPx() } * 2
                val cardPadding = with(density) { 12.dp.roundToPx() } * 2
                val itemWidth = (constraints.maxWidth - transcriptPadding).coerceAtLeast(0)
                val bubbleWidth = (itemWidth * 0.9f).roundToInt()
                val textWidth = (bubbleWidth - cardPadding).coerceAtLeast(0)
                val measureContext = TranscriptMeasureContext(
                    width = itemWidth,
                    density = density.density,
                    fontScale = density.fontScale,
                    layoutDirection = layoutDirection,
                    textMeasurer = textMeasurer,
                    styles = transcriptTextStyles,
                )
                if (transcriptMeasureCache.styles != transcriptTextStyles) {
                    transcriptMeasureCache.styles = transcriptTextStyles
                    transcriptMeasureCache.documents.clear()
                }
                if (transcriptMeasureCache.context != measureContext) {
                    transcriptMeasureCache.context = measureContext
                    transcriptMeasureCache.rows.clear()
                }
                val retainedRows = transcriptRows.toSet()
                transcriptMeasureCache.documents.keys.retainAll(retainedRows)
                transcriptMeasureCache.rows.keys.retainAll(retainedRows)
                val waitingSpacing = with(density) { 4.dp.roundToPx() }
                val attachmentControlHeight = with(density) { AttachmentControlHeight.roundToPx() }
                val attachmentTopSpacing = with(density) { AttachmentTopSpacing.roundToPx() }
                val imagePreviewHeight = with(density) { InlineImagePreviewHeight.roundToPx() }
                val imageDownloadSpacing = with(density) { ImageDownloadSpacing.roundToPx() }
                fun measuredText(
                    row: TranscriptRow,
                    content: String,
                    markdown: Boolean,
                    width: Int,
                ) = measureChatText(
                    document = transcriptMeasureCache.documents.getOrPut(row) {
                        buildChatText(content, markdown, transcriptTextStyles)
                    },
                    maxWidth = width,
                    textMeasurer = textMeasurer,
                    styles = transcriptTextStyles,
                    density = density,
                )
                for (row in transcriptRows) {
                    if (row in transcriptMeasureCache.rows) continue
                    transcriptMeasureCache.rows[row] = when (row) {
                        TranscriptRow.BottomAnchor -> MeasuredTranscriptRow(
                            text = null,
                            height = with(density) { 1.dp.roundToPx() },
                        )
                        TranscriptRow.Empty -> {
                            val text = measuredText(
                                row,
                                "Start a conversation with Pi.",
                                false,
                                itemWidth,
                            )
                            MeasuredTranscriptRow(text, text.height)
                        }
                        is TranscriptRow.Outgoing -> {
                            val text = measuredText(row, row.message.text, false, textWidth)
                            val waitingHeight = textMeasurer.measure(
                                text = "Waiting for Pi",
                                style = MaterialTheme.typography.labelSmall,
                                constraints = Constraints(maxWidth = textWidth),
                            ).size.height
                            MeasuredTranscriptRow(
                                text = text,
                                height = cardPadding + text.height + waitingSpacing + waitingHeight,
                            )
                        }
                        is TranscriptRow.Partial -> {
                            val text = measuredText(row, row.text, false, textWidth)
                            MeasuredTranscriptRow(text, cardPadding + text.height)
                        }
                        is TranscriptRow.Message -> {
                            val text = measuredText(
                                row,
                                row.message.text,
                                row.message.role != ChatRole.User,
                                textWidth,
                            )
                            val timestampHeight = row.timestamp?.let { timestamp ->
                                textMeasurer.measure(
                                    text = timestamp,
                                    style = MaterialTheme.typography.labelSmall,
                                    constraints = Constraints(maxWidth = textWidth),
                                ).size.height
                            } ?: 0
                            val attachmentHeight = when (row.message.attachment?.kind) {
                                null -> 0
                                AttachmentKind.File -> attachmentTopSpacing + attachmentControlHeight
                                AttachmentKind.Image -> attachmentTopSpacing + imagePreviewHeight +
                                    imageDownloadSpacing + attachmentControlHeight
                            }
                            MeasuredTranscriptRow(
                                text = text,
                                height = cardPadding + text.height + timestampHeight + attachmentHeight,
                            )
                        }
                    }
                }
                val geometry = TranscriptGeometry(
                    itemHeights = transcriptRows.map { transcriptMeasureCache.rows.getValue(it).height },
                    itemSpacing = with(density) { 12.dp.roundToPx() },
                    beforeContentPadding = with(density) { 16.dp.roundToPx() },
                    afterContentPadding = with(density) { 3.dp.roundToPx() },
                    viewportSize = constraints.maxHeight,
                )
                Box(Modifier.fillMaxSize()) {
                    SelectionContainer(transcriptSelectionState, Modifier.fillMaxSize()) {
                        LazyColumn(
                            state = listState,
                            modifier = Modifier.fillMaxSize().then(scrollMotion.modifier),
                            contentPadding = PaddingValues(
                                start = 16.dp,
                                top = 16.dp,
                                end = 16.dp,
                                bottom = 3.dp,
                            ),
                            reverseLayout = true,
                            verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.Bottom),
                            flingBehavior = scrollMotion.flingBehavior,
                        ) {
                            items(
                                transcriptRows,
                                key = TranscriptRow::key,
                            ) { row ->
                                val measured = transcriptMeasureCache.rows.getValue(row)
                                val attachmentDownload = if (row is TranscriptRow.Message) {
                                    state.attachmentDownloads[
                                        AttachmentDownloadKey(sessionId, row.message.entryId)
                                    ]
                                } else {
                                    null
                                }
                                Box(
                                    Modifier
                                        .fillMaxWidth()
                                        .height(with(density) { measured.height.toDp() }),
                                ) {
                                    TranscriptRowContent(
                                        row = row,
                                        measured = measured,
                                        styles = transcriptTextStyles,
                                        controller = controller,
                                        settings = state.settings,
                                        sessionId = sessionId,
                                        attachmentDownload = attachmentDownload,
                                        selectionState = transcriptSelectionState,
                                    )
                                }
                            }
                        }
                    }
                    TranscriptScrollbar(
                        state = listState,
                        geometry = geometry,
                        modifier = Modifier
                            .align(Alignment.CenterEnd)
                            .fillMaxHeight()
                            .padding(horizontal = 2.dp, vertical = 4.dp),
                    )
                    if (showScrollToBottom) {
                        FilledTonalIconButton(
                            onClick = { listState.requestScrollToItem(0) },
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
                state.extensionWidgets[sessionId]
                    .orEmpty()
                    .toSortedMap()
                    .values
                    .filter { widget -> widget.placement != "belowEditor" }
                    .forEach { widget ->
                        Surface(
                            color = MaterialTheme.colorScheme.secondaryContainer,
                            shape = MaterialTheme.shapes.small,
                        ) {
                            Text(
                                widget.lines.joinToString("\n"),
                                modifier = Modifier.fillMaxWidth().padding(8.dp),
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                    }
                if (suggestions.isNotEmpty()) {
                    Surface(
                        color = MaterialTheme.colorScheme.surfaceVariant,
                        shape = MaterialTheme.shapes.medium,
                        tonalElevation = 6.dp,
                    ) {
                        Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                            suggestions.forEachIndexed { index, suggestion ->
                                Row(
                                    Modifier
                                        .fillMaxWidth()
                                        .background(
                                            if (index == selectedSuggestion.coerceIn(
                                                    0,
                                                    suggestions.lastIndex,
                                                )
                                            ) {
                                                MaterialTheme.colorScheme.secondaryContainer
                                            } else {
                                                Color.Transparent
                                            },
                                        )
                                        .clickable { applySuggestion(suggestion) }
                                        .padding(horizontal = 12.dp, vertical = 7.dp),
                                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Text(
                                        suggestion.label,
                                        modifier = Modifier.weight(1f),
                                        style = MaterialTheme.typography.bodyMedium,
                                        fontFamily = FontFamily.Monospace,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                    suggestion.description?.let { description ->
                                        Text(
                                            description,
                                            modifier = Modifier.weight(1.4f),
                                            style = MaterialTheme.typography.labelSmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                            maxLines = 2,
                                            overflow = TextOverflow.Ellipsis,
                                        )
                                    }
                                }
                            }
                        }
                    }
                } else if (
                    editorValue.text.startsWith('/') &&
                    sessionId in state.loadingCommands
                ) {
                    Surface(
                        color = MaterialTheme.colorScheme.surfaceVariant,
                        shape = MaterialTheme.shapes.medium,
                    ) {
                        Row(
                            Modifier.fillMaxWidth().padding(10.dp),
                            horizontalArrangement = Arrangement.spacedBy(10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                            Text("Loading Pi commands", style = MaterialTheme.typography.bodySmall)
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
                                onClick = {
                                    listState.requestScrollToItem(0)
                                    controller.sendPrompt()
                                },
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
                    modifier = Modifier
                        .focusRequester(editorFocusRequester)
                        .fillMaxWidth()
                        .onClipboardImagePaste(canAttach, controller::attachClipboardImage)
                        .onPreviewKeyEvent { event ->
                            when {
                                suggestions.isNotEmpty() && event.key == Key.DirectionDown -> {
                                    if (event.type == KeyEventType.KeyDown) {
                                        selectedSuggestion = (selectedSuggestion + 1)
                                            .coerceAtMost(suggestions.lastIndex)
                                    }
                                    true
                                }
                                suggestions.isNotEmpty() && event.key == Key.DirectionUp -> {
                                    if (event.type == KeyEventType.KeyDown) {
                                        selectedSuggestion = (selectedSuggestion - 1).coerceAtLeast(0)
                                    }
                                    true
                                }
                                suggestions.isNotEmpty() && event.key == Key.Tab -> {
                                    if (event.type == KeyEventType.KeyDown) {
                                        applySuggestion(
                                            suggestions[selectedSuggestion.coerceIn(
                                                0,
                                                suggestions.lastIndex,
                                            )],
                                        )
                                    }
                                    true
                                }
                                suggestions.isNotEmpty() &&
                                    PlatformServices.platformName != "android" &&
                                    event.key == Key.Enter &&
                                    !event.isShiftPressed -> {
                                    if (event.type == KeyEventType.KeyDown) {
                                        applySuggestion(
                                            suggestions[selectedSuggestion.coerceIn(
                                                0,
                                                suggestions.lastIndex,
                                            )],
                                        )
                                    }
                                    true
                                }
                                PlatformServices.platformName == "android" ||
                                    event.key != Key.Enter -> false
                                event.isShiftPressed -> {
                                    if (event.type == KeyEventType.KeyDown) {
                                        val start = minOf(
                                            editorValue.selection.start,
                                            editorValue.selection.end,
                                        )
                                        val end = maxOf(
                                            editorValue.selection.start,
                                            editorValue.selection.end,
                                        )
                                        val updated = editorValue.text.replaceRange(start, end, "\n")
                                        editorValue = TextFieldValue(
                                            updated,
                                            TextRange(start + 1),
                                        )
                                        controller.setDraft(sessionId, updated)
                                    }
                                    true
                                }
                                else -> {
                                    if (event.type == KeyEventType.KeyDown && canSend) {
                                        listState.requestScrollToItem(0)
                                        controller.sendPrompt()
                                    }
                                    true
                                }
                            }
                        },
                )
                state.extensionWidgets[sessionId]
                    .orEmpty()
                    .toSortedMap()
                    .values
                    .filter { widget -> widget.placement == "belowEditor" }
                    .forEach { widget ->
                        Surface(
                            color = MaterialTheme.colorScheme.secondaryContainer,
                            shape = MaterialTheme.shapes.small,
                        ) {
                            Text(
                                widget.lines.joinToString("\n"),
                                modifier = Modifier.fillMaxWidth().padding(8.dp),
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                    }
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

private val CloseIcon = ImageVector.Builder(
    name = "Close",
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 24f,
    viewportHeight = 24f,
).apply {
    path(fill = SolidColor(Color.Black)) {
        moveTo(18.3f, 5.71f)
        lineTo(12f, 12f)
        lineTo(5.7f, 5.71f)
        lineTo(4.29f, 7.12f)
        lineTo(10.59f, 13.41f)
        lineTo(4.29f, 19.71f)
        lineTo(5.7f, 21.12f)
        lineTo(12f, 14.83f)
        lineTo(18.3f, 21.12f)
        lineTo(19.71f, 19.71f)
        lineTo(13.41f, 13.41f)
        lineTo(19.71f, 7.12f)
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
