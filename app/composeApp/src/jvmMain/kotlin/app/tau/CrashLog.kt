package app.tau

import java.io.File
import java.io.FileOutputStream
import java.io.PrintWriter
import java.io.Writer
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.Collections
import java.util.IdentityHashMap
import java.util.UUID

internal class CrashLog(private val directory: File) {
    private val lock = Any()
    private val maxBytes = 24 * 1024

    fun write(thread: Thread, failure: Throwable) {
        val reportId = UUID.randomUUID().toString()
        val trace = StringBuilder("Tau ${PlatformServices.appVersion} crash $reportId at ${System.currentTimeMillis()}\nThread: ${thread.name.take(128)}\n")
        val writer = object : Writer() {
            override fun write(buffer: CharArray, offset: Int, length: Int) {
                trace.append(buffer, offset, length.coerceAtMost((64 * 1024 - trace.length).coerceAtLeast(0)))
            }
            override fun write(text: String, offset: Int, length: Int) {
                trace.append(text, offset, offset + length.coerceAtMost((64 * 1024 - trace.length).coerceAtLeast(0)))
            }
            override fun flush() = Unit
            override fun close() = Unit
        }
        failure.printStackTrace(PrintWriter(writer))
        if (trace.length == 64 * 1024) trace.append("\n[trace truncated]\n")
        val records = mutableListOf("client-crash.log" to trace.toString().toByteArray(Charsets.UTF_8))
        try {
            val seen = Collections.newSetFromMap(IdentityHashMap<Throwable, Boolean>())
            val exceptions = mutableListOf<CrashCause>()
            val rangePattern = Regex("""Start\((-?\d+)\) or End\((-?\d+)\) is out of range \[0\.\.(-?\d+)\), or start > end!""")
            var cause: Throwable? = failure
            while (cause != null && exceptions.size < 4 && seen.add(cause)) {
                val frames = cause.stackTrace
                val range = if (cause is IllegalArgumentException && frames.any {
                    it.className == "androidx.compose.ui.text.MultiParagraph" && it.methodName == "getPathForRange"
                }) cause.message?.takeIf { it.length <= 200 }?.let(rangePattern::matchEntire)
                    ?.groupValues?.drop(1)?.mapNotNull(String::toIntOrNull)?.takeIf {
                        it.size == 3 && it[2] >= 0 && (it[0] < 0 || it[0] > it[1] || it[1] > it[2])
                    }
                    ?.let { CrashRange(it[0], it[1], it[2]) } else null
                exceptions += CrashCause(
                    exceptionClass = cause.javaClass.name.take(192),
                    stack = frames.take(if (exceptions.isEmpty()) 48 else 12).map { frame ->
                        CrashFrame(frame.className.take(192), frame.methodName.take(192), frame.fileName?.take(192), frame.lineNumber)
                    },
                    selectionRange = range,
                )
                cause = cause.cause
            }
            val first = exceptions.first()
            var report = CrashReport(
                schema = 2,
                reportId = reportId,
                platform = PlatformServices.platformName,
                appVersion = PlatformServices.appVersion,
                osVersion = PlatformServices.osVersion.take(192),
                thread = thread.name.take(128),
                exceptionClass = first.exceptionClass,
                stack = first.stack,
                selectionRange = first.selectionRange,
                causes = exceptions.drop(1),
            )
            var encoded = TauJson.encodeToString(report).toByteArray(Charsets.UTF_8)
            while (encoded.size >= maxBytes) {
                report = when {
                    report.stack.isNotEmpty() -> report.copy(stack = report.stack.dropLast(1))
                    report.causes.isNotEmpty() -> report.copy(causes = report.causes.dropLast(1))
                    else -> error("Crash report headers exceed $maxBytes bytes")
                }
                encoded = TauJson.encodeToString(report).toByteArray(Charsets.UTF_8)
            }
            records += "client-crash.pending.json" to encoded
        } catch (error: Throwable) {
            System.err.println("Tau could not encode the crash report")
            error.printStackTrace()
        }
        synchronized(lock) {
            for ((name, bytes) in records) {
                val target = File(directory, name).toPath()
                if (name == "client-crash.pending.json" && Files.exists(target)) continue
                val temporary = File(directory, "$name.tmp").toPath()
                try {
                    Files.createDirectories(directory.toPath())
                    FileOutputStream(temporary.toFile()).use { output ->
                        output.write(bytes)
                        output.fd.sync()
                    }
                    try {
                        Files.move(temporary, target, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE)
                    } catch (_: AtomicMoveNotSupportedException) {
                        Files.move(temporary, target, StandardCopyOption.REPLACE_EXISTING)
                    }
                } catch (error: Throwable) {
                    System.err.println("Tau could not save $name")
                    error.printStackTrace()
                } finally {
                    try {
                        Files.deleteIfExists(temporary)
                    } catch (error: Throwable) {
                        System.err.println("Tau could not remove $temporary")
                        error.printStackTrace()
                    }
                }
            }
        }
    }

    fun pendingReport(): String? = synchronized(lock) {
        val path = File(directory, "client-crash.pending.json").toPath()
        if (!Files.exists(path)) return@synchronized null
        try {
            require(Files.size(path) in 1 until maxBytes.toLong()) { "Invalid pending crash report size" }
            val encoded = path.toFile().readText(Charsets.UTF_8)
            TauJson.encodeToString(TauJson.decodeFromString<CrashReport>(encoded))
        } catch (error: Throwable) {
            System.err.println("Tau could not read the pending crash report")
            error.printStackTrace()
            null
        }
    }

    fun clearPendingReport(expected: String) {
        synchronized(lock) {
            if (pendingReport() == expected) Files.deleteIfExists(File(directory, "client-crash.pending.json").toPath())
        }
    }
}
