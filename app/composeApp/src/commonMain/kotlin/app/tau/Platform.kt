package app.tau

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import io.ktor.client.engine.HttpClientEngine
import kotlinx.serialization.Serializable

const val MaxUploadBytes = 50_000_000
const val MaxUploadFiles = 8

@Serializable
data class ConnectionSettings(
    val serverUrl: String = "http://vibe:8787",
    val token: String = "",
)

data class PickedFile(val name: String, val bytes: ByteArray)

expect object PlatformServices {
    val platformName: String
    val appVersion: String
    val osVersion: String

    fun loadConnection(): ConnectionSettings
    fun saveConnection(settings: ConnectionSettings)
    fun installCrashHandler()
    fun pendingCrashReport(): String?
    fun clearPendingCrashReport()
    suspend fun pickFiles(): List<PickedFile>
    suspend fun readDroppedFiles(fileUris: List<String>): List<PickedFile>
    fun saveDownload(fileName: String, bytes: ByteArray): String
}

expect fun Modifier.onSecondaryClick(onClick: (Offset) -> Unit): Modifier

@Composable
expect fun Modifier.onFilesDropped(
    enabled: Boolean,
    onDraggingChanged: (Boolean) -> Unit,
    onDrop: (List<String>) -> Unit,
): Modifier

expect fun Modifier.onInterruptShortcut(enabled: Boolean, onInterrupt: () -> Unit): Modifier

@Composable
expect fun PlatformBackHandler(enabled: Boolean, onBack: () -> Unit)

expect fun platformHttpEngine(): HttpClientEngine
