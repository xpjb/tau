package app.tau

import android.content.Context
import android.os.Build
import android.os.Process
import io.ktor.client.engine.HttpClientEngine
import io.ktor.client.engine.okhttp.OkHttp
import java.io.File
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.system.exitProcess
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString

internal object TauAndroidContext {
    private var applicationContext: Context? = null

    fun initialize(context: Context) {
        applicationContext = context.applicationContext
    }

    fun require(): Context = checkNotNull(applicationContext) { "Tau Android context is not ready" }
}

actual object PlatformServices {
    private val installed = AtomicBoolean(false)
    private val fileLock = Any()

    actual val platformName: String = "android"
    actual val appVersion: String = BuildConfig.VERSION_NAME
    actual val osVersion: String = "Android ${Build.VERSION.RELEASE} (SDK ${Build.VERSION.SDK_INT})"

    actual fun loadConnection(): ConnectionSettings {
        val preferences = TauAndroidContext.require()
            .getSharedPreferences("connection", Context.MODE_PRIVATE)
        return ConnectionSettings(
            serverUrl = preferences.getString("serverUrl", "http://vibe:8787")
                ?: "http://vibe:8787",
            token = preferences.getString("token", "").orEmpty(),
        )
    }

    actual fun saveConnection(settings: ConnectionSettings) {
        TauAndroidContext.require()
            .getSharedPreferences("connection", Context.MODE_PRIVATE)
            .edit()
            .putString("serverUrl", settings.serverUrl)
            .putString("token", settings.token)
            .apply()
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
                        val pending = File(TauAndroidContext.require().filesDir, "client-crash.pending.json")
                        if (!pending.exists()) {
                            val temporary = File(pending.parentFile, ".${pending.name}.tmp")
                            temporary.writeText(encoded)
                            if (!temporary.renameTo(pending)) temporary.delete()
                        }
                    }
                }
            } catch (_: Throwable) {
            } finally {
                if (previous != null) {
                    previous.uncaughtException(thread, throwable)
                } else {
                    Process.killProcess(Process.myPid())
                    exitProcess(10)
                }
            }
        }
    }

    actual fun pendingCrashReport(): String? = synchronized(fileLock) {
        val pending = File(TauAndroidContext.require().filesDir, "client-crash.pending.json")
        if (!pending.isFile || pending.length() > 24 * 1024) return@synchronized null
        runCatching {
            TauJson.encodeToString(TauJson.decodeFromString<CrashReport>(pending.readText()))
        }.getOrNull()
    }

    actual fun clearPendingCrashReport() {
        synchronized(fileLock) {
            File(TauAndroidContext.require().filesDir, "client-crash.pending.json").delete()
        }
    }
}

actual fun platformHttpEngine(): HttpClientEngine = OkHttp.create()
