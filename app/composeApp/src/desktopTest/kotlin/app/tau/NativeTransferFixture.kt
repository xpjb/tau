package app.tau

import java.nio.file.Files
import java.util.concurrent.TimeUnit
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

internal class NativeTransferFixture : AutoCloseable {
    private val root = Files.createTempDirectory("tau-native-provider").toFile()
    private val process = ProcessBuilder(checkNotNull(System.getProperty("tau.transfer.fixture")))
        .redirectError(ProcessBuilder.Redirect.INHERIT).start()
    private val input = process.outputStream.bufferedWriter()
    private val output = process.inputStream.bufferedReader()

    @Synchronized
    fun offer(nodeId: String, bytes: ByteArray, slow: Boolean = false): String {
        val file = Files.createTempFile(root.toPath(), "source-", ".bin").toFile()
        file.writeBytes(bytes)
        input.write(buildJsonObject {
            put("nodeId", nodeId)
            put("path", file.absolutePath)
            put("slow", slow)
        }.toString())
        input.newLine()
        input.flush()
        return checkNotNull(output.readLine()) { "Native provider stopped before returning a grant" }
    }

    override fun close() {
        try {
            input.close()
            if (!process.waitFor(10, TimeUnit.SECONDS)) {
                process.destroyForcibly()
                check(process.waitFor(5, TimeUnit.SECONDS)) { "Native provider did not stop" }
            }
            output.close()
            check(process.exitValue() == 0) { "Native provider failed: ${process.exitValue()}" }
        } finally {
            root.deleteRecursively()
        }
    }
}
