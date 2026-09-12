package app.tau

import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.server.cio.CIO
import io.ktor.server.engine.embeddedServer
import io.ktor.server.request.receiveText
import io.ktor.server.response.respond
import io.ktor.server.routing.post
import io.ktor.server.routing.routing
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.PrintStream
import java.net.URLClassLoader
import java.nio.file.Files
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class CrashLogTest {
    @Test
    fun retainsDiagnosticsAndUploadsOnlyBoundedSafeReports() = runBlocking {
        val home = System.getenv(if (System.getProperty("os.name").lowercase().contains("windows")) "LOCALAPPDATA" else "XDG_DATA_HOME")
        assertEquals("desktopTest", File(assertNotNull(home)).parentFile.name, "Use Gradle's isolated test store")
        val directory = File(PlatformServices.transcriptDatabasePath).parentFile
        val log = CrashLog(directory)
        val local = File(directory, "client-crash.log")
        val pendingFile = File(directory, "client-crash.pending.json")
        val temporary = Files.createTempDirectory("tau-crash-test").toFile()
        val received = Channel<Pair<String?, String>>(2)
        val attempts = AtomicInteger()
        val server = embeddedServer(CIO, host = "127.0.0.1", port = 0) {
            routing {
                post("/v1/telemetry/crash") {
                    received.send(call.request.headers[HttpHeaders.Authorization] to call.receiveText())
                    call.respond(if (attempts.getAndIncrement() == 0) HttpStatusCode.ServiceUnavailable else HttpStatusCode.NoContent)
                }
            }
        }.start(wait = false)
        val client = TauClient()
        try {
            pendingFile.delete()
            val secret = "private chat and token must remain local"
            val range = IllegalArgumentException("Start(5) or End(0) is out of range [0..710), or start > end!")
            range.stackTrace = arrayOf(StackTraceElement("androidx.compose.ui.text.MultiParagraph", "getPathForRange", "MultiParagraph.kt", 1271))
            val failure = IllegalStateException(secret, range)
            failure.addSuppressed(IllegalStateException("suppressed private detail"))
            range.initCause(failure)
            log.write(Thread.currentThread(), failure)
            val payload = assertNotNull(log.pendingReport())
            val report = TauJson.decodeFromString<CrashReport>(payload)
            assertEquals(2, report.schema)
            assertEquals(CrashRange(5, 0, 710), report.causes.single().selectionRange)
            assertNull(report.selectionRange)
            assertFalse(payload.contains(secret))
            assertFalse(payload.contains("suppressed private detail"))
            val trace = local.readText()
            for (part in listOf(secret, report.reportId, "Caused by:", "Suppressed:", "CIRCULAR REFERENCE")) assertTrue(trace.contains(part), part)

            log.write(Thread.currentThread(), IllegalStateException("second $secret"))
            assertEquals(payload, log.pendingReport(), "Keep the first pending report")
            assertTrue(local.readText().contains("second $secret"), "Keep the latest full local trace")
            val port = server.engine.resolvedConnectors().single().port
            val settings = ConnectionSettings("http://127.0.0.1:$port", "test-token")
            repeat(2) { attempt ->
                client.uploadPendingCrash(settings)
                val (authorization, body) = withTimeout(5_000) { received.receive() }
                assertEquals("Bearer test-token", authorization)
                assertEquals(payload, body)
                if (attempt == 0) assertEquals(payload, log.pendingReport()) else assertNull(log.pendingReport())
            }
            assertTrue(local.readText().contains("second $secret"), "Upload must not erase the full trace")

            val large = IllegalArgumentException("🔒".repeat(100_000))
            large.stackTrace = Array(80) { StackTraceElement("界".repeat(192), "方".repeat(192), "文".repeat(192), it) }
            log.write(Thread.currentThread(), large)
            val bounded = assertNotNull(log.pendingReport())
            assertTrue(bounded.toByteArray().size < 24 * 1024)
            assertTrue(TauJson.decodeFromString<CrashReport>(bounded).stack.size in 1 until 48)
            assertTrue(local.length() <= 256 * 1024 + 64)
            assertTrue(local.readText().endsWith("[trace truncated]\n"))
            assertFalse(bounded.contains("🔒"))
            log.clearPendingReport(payload)
            assertEquals(bounded, log.pendingReport(), "A stale upload reply must not clear a newer crash")
            log.clearPendingReport(bounded)

            val broken = File(temporary, "broken")
            val blockedLocal = File(broken, "client-crash.log").apply { mkdirs() }
            val brokenLog = CrashLog(broken)
            val errors = ByteArrayOutputStream()
            val stderr = System.err
            try {
                System.setErr(PrintStream(errors, true, Charsets.UTF_8))
                brokenLog.write(Thread.currentThread(), failure)
                val brokenPayload = assertNotNull(brokenLog.pendingReport(), "A local trace failure must not discard telemetry")
                brokenLog.clearPendingReport(brokenPayload)
                assertTrue(blockedLocal.delete())
                assertTrue(File(broken, "client-crash.pending.json.tmp").mkdir())
                brokenLog.write(Thread.currentThread(), failure)
                assertNull(brokenLog.pendingReport())
                assertTrue(blockedLocal.readText().contains(secret), "Telemetry failure must not discard the local trace")
            } finally {
                System.setErr(stderr)
            }
            assertTrue(errors.toString(Charsets.UTF_8).contains("could not save client-crash.log"))
            assertTrue(errors.toString(Charsets.UTF_8).contains("could not save client-crash.pending.json"))

            val output = File(temporary, "process.log")
            val classpath = (listOf(System.getProperty("java.class.path")) +
                generateSequence(javaClass.classLoader) { it.parent }.filterIsInstance<URLClassLoader>()
                    .flatMap { it.urLs.asSequence() }.map { File(it.toURI()).path }.toList()).joinToString(File.pathSeparator)
            val process = ProcessBuilder(File(System.getProperty("java.home"), "bin/java").path,
                "-cp", classpath, "app.tau.CrashHandlerProbe")
                .redirectErrorStream(true).redirectOutput(output).apply {
                    environment()["XDG_DATA_HOME"] = temporary.path
                    environment()["LOCALAPPDATA"] = temporary.path
                }.start()
            try {
                assertTrue(process.waitFor(30, TimeUnit.SECONDS), "Crash handler did not finish")
                assertEquals(10, process.exitValue(), output.readText())
                assertTrue(output.readText().contains("uncaught diagnostics probe"))
                val pending = temporary.walkTopDown().single { it.name == "client-crash.pending.json" }
                assertEquals("java.lang.IllegalArgumentException", TauJson.decodeFromString<CrashReport>(pending.readText()).causes.single().exceptionClass)
                assertTrue(temporary.walkTopDown().single { it.name == "client-crash.log" && it.isFile && it.parentFile != broken }
                    .readText().contains("uncaught diagnostics probe"))
            } finally {
                process.destroyForcibly()
            }
        } finally {
            client.close()
            server.stop(0, 1_000)
            pendingFile.delete()
            local.delete()
            temporary.deleteRecursively()
        }
    }
}

object CrashHandlerProbe {
    @JvmStatic
    fun main(args: Array<String>) {
        PlatformServices.installCrashHandler()
        PlatformServices.installCrashHandler()
        throw IllegalStateException("uncaught diagnostics probe", IllegalArgumentException("cause probe"))
    }
}
