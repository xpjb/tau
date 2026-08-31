package app.tau

import io.ktor.client.engine.HttpClientEngine
import kotlinx.serialization.Serializable

@Serializable
data class ConnectionSettings(
    val serverUrl: String = "http://vibe:8787",
    val token: String = "",
)

expect object PlatformServices {
    val platformName: String
    val appVersion: String
    val osVersion: String

    fun loadConnection(): ConnectionSettings
    fun saveConnection(settings: ConnectionSettings)
    fun installCrashHandler()
    fun pendingCrashReport(): String?
    fun clearPendingCrashReport()
    fun saveDownload(fileName: String, bytes: ByteArray): String
}

expect fun platformHttpEngine(): HttpClientEngine
