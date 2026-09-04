package app.tau

import androidx.compose.animation.core.AnimationState
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.animateTo
import androidx.compose.animation.core.tween
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.LocalScrollbarStyle
import androidx.compose.foundation.VerticalScrollbar
import androidx.compose.foundation.draganddrop.dragAndDropTarget
import androidx.compose.foundation.gestures.FlingBehavior
import androidx.compose.foundation.gestures.ScrollScope
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.v2.ScrollbarAdapter
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draganddrop.DragAndDropEvent
import androidx.compose.ui.draganddrop.DragAndDropTarget
import androidx.compose.ui.draganddrop.DragData
import androidx.compose.ui.draganddrop.dragData
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.PointerButton
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.isSecondaryPressed
import androidx.compose.ui.input.pointer.onPointerEvent
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import io.ktor.client.engine.HttpClientEngine
import io.ktor.client.engine.cio.CIO
import java.awt.Desktop
import java.awt.FileDialog
import java.awt.Frame
import java.awt.Image
import java.awt.Toolkit
import java.awt.datatransfer.DataFlavor
import java.awt.datatransfer.StringSelection
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.net.URI
import java.nio.file.Files
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.attribute.PosixFilePermission
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import javax.imageio.ImageIO
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlin.math.sign
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString

actual object PlatformServices {
    private val installed = AtomicBoolean(false)
    private val fileLock = Any()
    private val dataDirectory: Path by lazy {
        val os = System.getProperty("os.name").orEmpty().lowercase()
        val path = if (os.contains("windows")) {
            Path.of(
                System.getenv("LOCALAPPDATA")
                    ?: Path.of(System.getProperty("user.home"), "AppData", "Local").toString(),
                "Tau",
                "data",
            )
        } else {
            Path.of(
                System.getenv("XDG_DATA_HOME")
                    ?: Path.of(System.getProperty("user.home"), ".local", "share").toString(),
                "Tau",
            )
        }
        Files.createDirectories(path)
        runCatching {
            Files.setPosixFilePermissions(
                path,
                setOf(
                    PosixFilePermission.OWNER_READ,
                    PosixFilePermission.OWNER_WRITE,
                    PosixFilePermission.OWNER_EXECUTE,
                ),
            )
        }
        path
    }

    actual val platformName: String = if (
        System.getProperty("os.name").orEmpty().contains("Windows", ignoreCase = true)
    ) {
        "windows"
    } else {
        "desktop"
    }
    actual val appVersion: String = TauClientVersion
    actual val osVersion: String = (
        System.getProperty("os.name").orEmpty() + " " + System.getProperty("os.version").orEmpty()
    ).trim()
    actual val thumbnailCacheDirectory: String by lazy {
        dataDirectory.resolve("image-thumbnails").toString()
    }

    actual fun loadConnection(): ConnectionSettings {
        val path = dataDirectory.resolve("connection.json")
        if (!Files.isRegularFile(path) || runCatching { Files.size(path) }.getOrDefault(0) > 16 * 1024) {
            return ConnectionSettings()
        }
        return runCatching {
            TauJson.decodeFromString<ConnectionSettings>(Files.readString(path))
        }.getOrElse { ConnectionSettings() }
    }

    actual fun saveConnection(settings: ConnectionSettings) {
        val target = dataDirectory.resolve("connection.json")
        val temporary = dataDirectory.resolve(".connection.json.tmp")
        Files.writeString(temporary, TauJson.encodeToString(settings))
        runCatching {
            Files.setPosixFilePermissions(
                temporary,
                setOf(PosixFilePermission.OWNER_READ, PosixFilePermission.OWNER_WRITE),
            )
        }
        try {
            Files.move(
                temporary,
                target,
                StandardCopyOption.ATOMIC_MOVE,
                StandardCopyOption.REPLACE_EXISTING,
            )
        } catch (_: Throwable) {
            Files.move(temporary, target, StandardCopyOption.REPLACE_EXISTING)
        }
    }

    actual fun installCrashHandler() {
        if (!installed.compareAndSet(false, true)) return
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, throwable ->
            try {
                val report = CrashReport(
                    reportId = UUID.randomUUID().toString(),
                    platform = platformName,
                    appVersion = appVersion,
                    osVersion = osVersion.take(192),
                    thread = thread.name.take(128),
                    exceptionClass = throwable.javaClass.name.take(192),
                    stack = throwable.stackTrace.take(48).map { frame ->
                        CrashFrame(
                            className = frame.className.take(192),
                            methodName = frame.methodName.take(192),
                            fileName = frame.fileName?.take(192),
                            lineNumber = frame.lineNumber,
                        )
                    },
                )
                val encoded = TauJson.encodeToString(report)
                if (encoded.encodeToByteArray().size <= 24 * 1024) {
                    synchronized(fileLock) {
                        val pending = dataDirectory.resolve("client-crash.pending.json")
                        if (!Files.exists(pending)) {
                            val temporary = dataDirectory.resolve(".client-crash.pending.json.tmp")
                            Files.writeString(temporary, encoded)
                            try {
                                Files.move(temporary, pending, StandardCopyOption.ATOMIC_MOVE)
                            } catch (_: Throwable) {
                                Files.deleteIfExists(temporary)
                            }
                        }
                    }
                }
            } catch (_: Throwable) {
            } finally {
                if (previous != null) {
                    previous.uncaughtException(thread, throwable)
                } else {
                    kotlin.system.exitProcess(10)
                }
            }
        }
    }

    actual fun pendingCrashReport(): String? = synchronized(fileLock) {
        val pending = dataDirectory.resolve("client-crash.pending.json")
        if (!Files.isRegularFile(pending) || runCatching { Files.size(pending) }.getOrDefault(0) > 24 * 1024) {
            return@synchronized null
        }
        runCatching {
            TauJson.encodeToString(TauJson.decodeFromString<CrashReport>(Files.readString(pending)))
        }.getOrNull()
    }

    actual fun clearPendingCrashReport() {
        synchronized(fileLock) {
            Files.deleteIfExists(dataDirectory.resolve("client-crash.pending.json"))
        }
    }

    actual fun copyText(text: String) {
        Toolkit.getDefaultToolkit().systemClipboard.setContents(StringSelection(text), null)
    }

    actual fun formatMessageTime(timestampMs: Long): String = runCatching {
        DateTimeFormatter
            .ofLocalizedTime(FormatStyle.SHORT)
            .format(Instant.ofEpochMilli(timestampMs).atZone(ZoneId.systemDefault()))
    }.getOrDefault("")

    actual suspend fun pickFiles(): List<PickedFile> {
        val paths = FileDialog(null as Frame?, "Attach files", FileDialog.LOAD).apply {
            isMultipleMode = true
            isVisible = true
        }.files.map { it.toPath() }
        return withContext(Dispatchers.IO) { readFiles(paths) }
    }

    actual suspend fun readDroppedFiles(fileUris: List<String>): List<PickedFile> =
        withContext(Dispatchers.IO) {
            readFiles(fileUris.map { Path.of(URI(it)) })
        }

    private fun readFiles(paths: List<Path>): List<PickedFile> {
        if (paths.size > MaxUploadFiles) {
            error("Attach at most $MaxUploadFiles files at once")
        }
        var total = 0L
        return paths.map { path ->
            val name = path.fileName?.toString().orEmpty().ifBlank { "attachment" }
            if (!Files.isRegularFile(path)) error("$name is not a regular file")
            val size = Files.size(path)
            if (size == 0L) error("$name is empty")
            total += size
            if (total > MaxUploadBytes) {
                error("Attached files exceed Tau's $MaxUploadBytes byte limit")
            }
            PickedFile(name, Files.readAllBytes(path))
        }
    }

    actual fun saveDownload(fileName: String, bytes: ByteArray): SavedDownload {
        val safeName = fileName
            .substringAfterLast('/')
            .substringAfterLast('\\')
            .map { character ->
                if (character.code < 32 || character in "<>:\"/\\|?*") '_' else character
            }
            .joinToString("")
            .take(160)
            .ifBlank { "tau-attachment" }
        val directory = Path.of(System.getProperty("user.home"), "Downloads", "Tau")
        Files.createDirectories(directory)
        val extensionIndex = safeName.lastIndexOf('.').takeIf { it > 0 } ?: safeName.length
        val stem = safeName.substring(0, extensionIndex)
        val extension = safeName.substring(extensionIndex)
        var target = directory.resolve(safeName)
        var suffix = 2
        while (Files.exists(target)) {
            target = directory.resolve("$stem ($suffix)$extension")
            suffix += 1
        }
        val temporary = directory.resolve(".${target.fileName}.tmp-${UUID.randomUUID()}")
        Files.write(temporary, bytes)
        try {
            Files.move(temporary, target, StandardCopyOption.ATOMIC_MOVE)
        } catch (_: Throwable) {
            Files.move(temporary, target)
        }
        return SavedDownload(
            location = target.toString(),
            reference = target.toString(),
            mimeType = Files.probeContentType(target) ?: "application/octet-stream",
        )
    }

    actual fun openDownload(download: SavedDownload) {
        val path = Path.of(download.reference)
        check(Files.isRegularFile(path)) { "The downloaded file no longer exists." }
        if (platformName == "windows") {
            val launcher = System.getenv("LOCALAPPDATA")
                ?.let { directory -> Path.of(directory, "Tau", "Tau.exe") }
            if (launcher != null && Files.isRegularFile(launcher)) {
                ProcessBuilder(launcher.toString(), "--open", path.toString()).start()
                return
            }
        }
        check(Desktop.isDesktopSupported()) { "Opening files is not supported on this desktop." }
        val desktop = Desktop.getDesktop()
        check(desktop.isSupported(Desktop.Action.OPEN)) {
            "Opening files is not supported on this desktop."
        }
        desktop.open(path.toFile())
    }
}

@OptIn(ExperimentalComposeUiApi::class)
actual fun Modifier.onSecondaryClick(onClick: (Offset) -> Unit): Modifier =
    onPointerEvent(PointerEventType.Press) { event ->
        if (event.buttons.isSecondaryPressed) {
            event.changes.firstOrNull()?.let { onClick(it.position) }
            event.changes.forEach { it.consume() }
        }
    }

@Composable
@OptIn(ExperimentalComposeUiApi::class, ExperimentalFoundationApi::class)
actual fun Modifier.onFilesDropped(
    enabled: Boolean,
    onDraggingChanged: (Boolean) -> Unit,
    onDrop: (List<String>) -> Unit,
): Modifier {
    val currentEnabled = rememberUpdatedState(enabled)
    val currentDraggingChanged = rememberUpdatedState(onDraggingChanged)
    val currentDrop = rememberUpdatedState(onDrop)
    val target = remember {
        object : DragAndDropTarget {
            override fun onEntered(event: DragAndDropEvent) {
                currentDraggingChanged.value(true)
            }

            override fun onExited(event: DragAndDropEvent) {
                currentDraggingChanged.value(false)
            }

            override fun onEnded(event: DragAndDropEvent) {
                currentDraggingChanged.value(false)
            }

            override fun onDrop(event: DragAndDropEvent): Boolean {
                currentDraggingChanged.value(false)
                val data = event.dragData() as? DragData.FilesList ?: return false
                val files = runCatching { data.readFiles() }.getOrElse { return false }
                if (files.isEmpty()) return false
                currentDrop.value(files)
                return true
            }
        }
    }
    return dragAndDropTarget(
        shouldStartDragAndDrop = { event ->
            currentEnabled.value && event.dragData() is DragData.FilesList
        },
        target = target,
    )
}

@Composable
actual fun Modifier.onClipboardImagePaste(
    enabled: Boolean,
    onPaste: (suspend () -> PickedFile) -> Unit,
): Modifier {
    val currentEnabled = rememberUpdatedState(enabled)
    val currentPaste = rememberUpdatedState(onPaste)
    return onPreviewKeyEvent { event ->
        if (!currentEnabled.value || event.key != Key.V || !event.isCtrlPressed) {
            false
        } else {
            val contents = runCatching {
                Toolkit.getDefaultToolkit().systemClipboard.getContents(null)
            }.getOrNull()
            if (
                contents == null ||
                !runCatching {
                    contents.isDataFlavorSupported(DataFlavor.imageFlavor)
                }.getOrDefault(false)
            ) {
                false
            } else {
                if (event.type == KeyEventType.KeyDown) {
                    currentPaste.value {
                        withContext(Dispatchers.IO) {
                            val image = contents.getTransferData(DataFlavor.imageFlavor) as? Image
                                ?: error("The clipboard image could not be read")
                            val width = image.getWidth(null)
                            val height = image.getHeight(null)
                            check(width > 0 && height > 0) {
                                "The clipboard image has invalid dimensions"
                            }
                            val buffered = if (image is BufferedImage) {
                                image
                            } else {
                                BufferedImage(width, height, BufferedImage.TYPE_INT_ARGB).also { target ->
                                    target.createGraphics().let { graphics ->
                                        try {
                                            graphics.drawImage(image, 0, 0, null)
                                        } finally {
                                            graphics.dispose()
                                        }
                                    }
                                }
                            }
                            val output = ByteArrayOutputStream()
                            check(ImageIO.write(buffered, "png", output)) {
                                "The clipboard image could not be encoded"
                            }
                            check(output.size().toLong() <= MaxUploadBytes) {
                                "The clipboard image exceeds Tau's $MaxUploadBytes byte limit"
                            }
                            PickedFile(
                                name = "clipboard-image-${System.currentTimeMillis()}.png",
                                bytes = output.toByteArray(),
                            )
                        }
                    }
                }
                true
            }
        }
    }
}

actual fun Modifier.onInterruptShortcut(enabled: Boolean, onInterrupt: () -> Unit): Modifier =
    onPreviewKeyEvent { event ->
        if (enabled && event.key == Key.Escape && event.type == KeyEventType.KeyDown) {
            onInterrupt()
            true
        } else {
            false
        }
    }

@OptIn(ExperimentalComposeUiApi::class)
@Composable
actual fun Modifier.onTranscriptAutoscroll(state: LazyListState): Modifier {
    var anchor by remember(state) { mutableStateOf<Offset?>(null) }
    var pointer by remember(state) { mutableStateOf(Offset.Zero) }
    var middlePressedAt by remember(state) { mutableStateOf<Long?>(null) }
    val density = LocalDensity.current
    val deadZone = with(density) { 6.dp.toPx() }
    val maxSpeed = with(density) { 7_200.dp.toPx() }
    val markerRadius = with(density) { 13.5.dp.toPx() }
    val markerColor = MaterialTheme.colorScheme.primary
    val markerBackground = MaterialTheme.colorScheme.surface.copy(alpha = 0.92f)

    LaunchedEffect(state, anchor) {
        var previousFrame = withFrameNanos { it }
        while (anchor != null) {
            val frame = withFrameNanos { it }
            val elapsedSeconds = (frame - previousFrame) / 1_000_000_000f
            previousFrame = frame
            val distance = pointer.y - checkNotNull(anchor).y
            val excess = abs(distance) - deadZone
            if (excess > 0f) {
                val speed = distance.sign * minOf(maxSpeed, excess * 30f)
                state.scrollBy(-speed * elapsedSeconds)
            }
        }
    }

    return onPointerEvent(PointerEventType.Move) { event ->
        if (anchor != null) event.changes.firstOrNull()?.let { pointer = it.position }
    }.onPointerEvent(PointerEventType.Scroll) {
        anchor = null
        middlePressedAt = null
    }.onPointerEvent(PointerEventType.Press) { event ->
        val position = event.changes.firstOrNull()?.position ?: return@onPointerEvent
        when (event.button) {
            PointerButton.Tertiary -> {
                if (anchor == null) {
                    anchor = position
                    pointer = position
                    middlePressedAt = System.nanoTime()
                } else {
                    anchor = null
                    middlePressedAt = null
                }
                event.changes.forEach { it.consume() }
            }
            PointerButton.Primary, PointerButton.Secondary -> if (anchor != null) {
                anchor = null
                middlePressedAt = null
                event.changes.forEach { it.consume() }
            }
        }
    }.onPointerEvent(PointerEventType.Release) { event ->
        if (event.button == PointerButton.Tertiary) {
            val pressedAt = middlePressedAt
            middlePressedAt = null
            if (pressedAt != null && System.nanoTime() - pressedAt >= 220_000_000L) {
                anchor = null
            }
            event.changes.forEach { it.consume() }
        }
    }.drawWithContent {
        drawContent()
        anchor?.let { position ->
            drawCircle(markerBackground, markerRadius, position)
            drawCircle(markerColor, markerRadius, position, style = Stroke(width = 2f))
            val tip = markerRadius * 0.58f
            val wing = markerRadius * 0.28f
            drawLine(markerColor, Offset(position.x, position.y - tip), Offset(position.x, position.y + tip))
            drawLine(
                markerColor,
                Offset(position.x, position.y - tip),
                Offset(position.x - wing, position.y - tip + wing),
            )
            drawLine(
                markerColor,
                Offset(position.x, position.y - tip),
                Offset(position.x + wing, position.y - tip + wing),
            )
            drawLine(
                markerColor,
                Offset(position.x, position.y + tip),
                Offset(position.x - wing, position.y + tip - wing),
            )
            drawLine(
                markerColor,
                Offset(position.x, position.y + tip),
                Offset(position.x + wing, position.y + tip - wing),
            )
        }
    }
}

private class TranscriptInputSource {
    var touchpad = false
}

internal fun transcriptTouchpadFlingProgress(fraction: Float): Float =
    1f - (1f - fraction) * (1f - fraction)

internal fun transcriptTouchpadFlingDurationMillis(
    velocity: Float,
    maximumVelocity: Float,
    deceleration: Float,
): Int = (abs(velocity.coerceIn(-maximumVelocity, maximumVelocity)) / deceleration * 1_000f)
    .roundToInt()
    .coerceIn(80, 1_600)

internal fun transcriptTouchpadFlingDistance(velocity: Float, durationMillis: Int): Float =
    velocity * durationMillis / 2_000f

private val TouchpadFlingEasing = Easing(::transcriptTouchpadFlingProgress)

private class TranscriptFlingBehavior(
    private val inputSource: TranscriptInputSource,
    private val maximumVelocity: Float,
    private val deceleration: Float,
) : FlingBehavior {
    override suspend fun ScrollScope.performFling(initialVelocity: Float): Float {
        if (!inputSource.touchpad) return initialVelocity
        val velocity = initialVelocity.coerceIn(-maximumVelocity, maximumVelocity)
        if (velocity == 0f) return 0f
        val durationMillis = transcriptTouchpadFlingDurationMillis(
            velocity,
            maximumVelocity,
            deceleration,
        )
        val distance = transcriptTouchpadFlingDistance(velocity, durationMillis)
        var previous = 0f
        var remainingVelocity = 0f
        AnimationState(initialValue = 0f, initialVelocity = velocity).animateTo(
            targetValue = distance,
            animationSpec = tween(durationMillis, easing = TouchpadFlingEasing),
        ) {
            val delta = value - previous
            val consumed = scrollBy(delta)
            previous = value
            if (abs(delta - consumed) > 0.5f) {
                remainingVelocity = this.velocity
                cancelAnimation()
            }
        }
        return remainingVelocity
    }
}

@Composable
@OptIn(ExperimentalComposeUiApi::class)
actual fun rememberTranscriptScrollMotion(): TranscriptScrollMotion {
    val density = LocalDensity.current
    val inputSource = remember { TranscriptInputSource() }
    val behavior = remember(density, inputSource) {
        TranscriptFlingBehavior(
            inputSource = inputSource,
            maximumVelocity = with(density) { 4_200.dp.toPx() },
            deceleration = with(density) { 3_000.dp.toPx() },
        )
    }
    val modifier = Modifier
        .onPointerEvent(PointerEventType.PanStart, PointerEventPass.Initial) {
            inputSource.touchpad = true
        }
        .onPointerEvent(PointerEventType.PanMove, PointerEventPass.Initial) {
            inputSource.touchpad = true
        }
        .onPointerEvent(PointerEventType.PanEnd, PointerEventPass.Initial) {
            inputSource.touchpad = true
        }
        .onPointerEvent(PointerEventType.Scroll, PointerEventPass.Initial) {
            inputSource.touchpad = false
        }
        .onPointerEvent(PointerEventType.Press, PointerEventPass.Initial) {
            inputSource.touchpad = false
        }
    return TranscriptScrollMotion(modifier, behavior)
}

@Composable
@OptIn(ExperimentalComposeUiApi::class)
actual fun TranscriptScrollbar(
    state: LazyListState,
    geometry: TranscriptGeometry,
    modifier: Modifier,
) {
    if (geometry.maxScrollOffset == 0.0) return
    val currentGeometry by rememberUpdatedState(geometry)
    val adapter = remember(state) {
        object : ScrollbarAdapter {
            override val viewportSize: Double
                get() = currentGeometry.viewportSize.toDouble()

            override val contentSize: Double
                get() = currentGeometry.contentSize

            override val scrollOffset: Double
                get() = currentGeometry.scrollOffset(
                    state.firstVisibleItemIndex,
                    state.firstVisibleItemScrollOffset,
                )

            override suspend fun scrollTo(scrollOffset: Double) {
                val position = currentGeometry.positionAt(scrollOffset)
                state.scrollToItem(position.index, position.scrollOffset)
            }
        }
    }
    VerticalScrollbar(
        adapter = adapter,
        modifier = modifier,
        reverseLayout = true,
        style = LocalScrollbarStyle.current.copy(
            thickness = 8.dp,
            unhoverColor = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.28f),
            hoverColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.72f),
        ),
    )
}

@Composable
actual fun PlatformBackHandler(enabled: Boolean, onBack: () -> Unit) = Unit

actual fun platformHttpEngine(): HttpClientEngine = CIO.create()
