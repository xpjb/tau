package app.tau

import io.ktor.client.engine.HttpClientEngine
import io.ktor.client.engine.cio.CIO
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.attribute.PosixFilePermission
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
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
    actual val appVersion: String = "0.1.0"
    actual val osVersion: String = (
        System.getProperty("os.name").orEmpty() + " " + System.getProperty("os.version").orEmpty()
    ).trim()

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
}

actual fun platformHttpEngine(): HttpClientEngine = CIO.create()
