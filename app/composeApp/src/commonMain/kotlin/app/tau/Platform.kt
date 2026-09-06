package app.tau

import androidx.compose.foundation.gestures.FlingBehavior
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import io.ktor.client.engine.HttpClientEngine
import kotlinx.serialization.Serializable

const val MaxUploadBytes = 50_000_000
const val MaxUploadFiles = 8
const val TauClientVersion = "0.5.3"
internal const val TauHeartbeatMillis = 15_000L

@Serializable
data class ConnectionSettings(
    val serverUrl: String = "http://vibe:8787",
    val token: String = "",
)

data class PickedFile(val name: String, val bytes: ByteArray)

data class SavedDownload(
    val location: String,
    val reference: String,
    val mimeType: String,
)

expect object PlatformServices {
    val platformName: String
    val appVersion: String
    val osVersion: String
    val thumbnailCacheDirectory: String
    val transcriptDatabasePath: String

    fun loadConnection(): ConnectionSettings
    fun saveConnection(settings: ConnectionSettings)
    fun installCrashHandler()
    fun pendingCrashReport(): String?
    fun clearPendingCrashReport()
    fun copyText(text: String)
    fun formatMessageTime(timestampMs: Long): String
    suspend fun pickFiles(): List<PickedFile>
    suspend fun readDroppedFiles(fileUris: List<String>): List<PickedFile>
    fun saveDownload(fileName: String, bytes: ByteArray): SavedDownload
    fun openDownload(download: SavedDownload)
    fun showDownload(download: SavedDownload)
    fun extractAndOpenDownload(download: SavedDownload)
}

expect fun Modifier.onSecondaryClick(onClick: (Offset) -> Unit): Modifier

@Composable
expect fun Modifier.onFilesDropped(
    enabled: Boolean,
    onDraggingChanged: (Boolean) -> Unit,
    onDrop: (List<String>) -> Unit,
): Modifier

@Composable
expect fun Modifier.onClipboardImagePaste(
    enabled: Boolean,
    onPaste: (suspend () -> PickedFile) -> Unit,
): Modifier

expect fun Modifier.onInterruptShortcut(enabled: Boolean, onInterrupt: () -> Unit): Modifier

@Composable
expect fun Modifier.onTranscriptAutoscroll(state: LazyListState): Modifier

class TranscriptScrollMotion(
    val modifier: Modifier,
    val flingBehavior: FlingBehavior,
)

@Composable
expect fun rememberTranscriptScrollMotion(): TranscriptScrollMotion

@Composable
expect fun TranscriptScrollbar(
    state: LazyListState,
    modifier: Modifier = Modifier,
)

@Composable
expect fun PlatformBackHandler(enabled: Boolean, onBack: () -> Unit)

expect fun platformHttpEngine(): HttpClientEngine
