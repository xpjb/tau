package app.tau

import androidx.compose.ui.Modifier
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
    fun saveDownload(fileName: String, bytes: ByteArray): String
}

expect fun Modifier.onSecondaryClick(onClick: () -> Unit): Modifier

expect fun platformHttpEngine(): HttpClientEngine
