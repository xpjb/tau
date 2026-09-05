package app.tau

import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.Process
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.text.format.DateFormat
import android.webkit.MimeTypeMap
import androidx.activity.compose.BackHandler
import androidx.activity.result.ActivityResultLauncher
import androidx.compose.foundation.gestures.ScrollableDefaults
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.core.content.FileProvider
import io.ktor.client.engine.HttpClientEngine
import io.ktor.client.engine.okhttp.OkHttp
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.Date
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.system.exitProcess
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString

internal object TauAndroidContext {
    private var applicationContext: Context? = null
    private var filePicker: ActivityResultLauncher<Array<String>>? = null
    private var fileSelection: CancellableContinuation<List<Uri>>? = null

    fun initialize(context: Context, picker: ActivityResultLauncher<Array<String>>) {
        applicationContext = context.applicationContext
        filePicker = picker
    }

    fun require(): Context = checkNotNull(applicationContext) { "Tau Android context is not ready" }

    suspend fun selectFiles(): List<Uri> = suspendCancellableCoroutine { continuation ->
        val picker = synchronized(this) {
            check(fileSelection == null) { "A file picker is already open" }
            val picker = checkNotNull(filePicker) { "Tau's file picker is not ready" }
            fileSelection = continuation
            picker
        }
        continuation.invokeOnCancellation {
            synchronized(this) {
                if (fileSelection === continuation) fileSelection = null
            }
        }
        try {
            picker.launch(arrayOf("*/*"))
        } catch (error: Throwable) {
            synchronized(this) {
                if (fileSelection === continuation) fileSelection = null
            }
            continuation.resumeWith(Result.failure(error))
        }
    }

    fun completeFileSelection(uris: List<Uri>) {
        val continuation = synchronized(this) {
            fileSelection.also { fileSelection = null }
        }
        if (continuation?.isActive == true) {
            continuation.resumeWith(Result.success(uris))
        }
    }
}

actual object PlatformServices {
    private val installed = AtomicBoolean(false)
    private val fileLock = Any()

    actual val platformName: String = "android"
    actual val appVersion: String = TauClientVersion
    actual val osVersion: String = "Android ${Build.VERSION.RELEASE} (SDK ${Build.VERSION.SDK_INT})"
    actual val thumbnailCacheDirectory: String
        get() = File(TauAndroidContext.require().cacheDir, "image-thumbnails").absolutePath

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

    actual fun copyText(text: String) {
        val clipboard = TauAndroidContext.require()
            .getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("Tau message", text))
    }

    actual fun formatMessageTime(timestampMs: Long): String = runCatching {
        DateFormat.getTimeFormat(TauAndroidContext.require()).format(Date(timestampMs))
    }.getOrDefault("")

    actual suspend fun pickFiles(): List<PickedFile> {
        val uris = TauAndroidContext.selectFiles()
        if (uris.size > MaxUploadFiles) {
            error("Attach at most $MaxUploadFiles files at once")
        }
        return withContext(Dispatchers.IO) {
            val resolver = TauAndroidContext.require().contentResolver
            var total = 0
            uris.map { uri ->
                val name = resolver.query(
                    uri,
                    arrayOf(OpenableColumns.DISPLAY_NAME),
                    null,
                    null,
                    null,
                )?.use { cursor ->
                    if (cursor.moveToFirst()) cursor.getString(0) else null
                } ?: uri.lastPathSegment ?: "attachment"
                val output = ByteArrayOutputStream()
                checkNotNull(resolver.openInputStream(uri)) { "Android could not open $name" }.use { input ->
                    val buffer = ByteArray(64 * 1024)
                    while (true) {
                        val count = input.read(buffer)
                        if (count < 0) break
                        total += count
                        if (total > MaxUploadBytes) {
                            error("Attached files exceed Tau's $MaxUploadBytes byte limit")
                        }
                        output.write(buffer, 0, count)
                    }
                }
                if (output.size() == 0) error("$name is empty")
                PickedFile(name, output.toByteArray())
            }
        }
    }

    actual suspend fun readDroppedFiles(fileUris: List<String>): List<PickedFile> = emptyList()

    actual fun saveDownload(fileName: String, bytes: ByteArray): SavedDownload {
        val safeName = fileName
            .substringAfterLast('/')
            .substringAfterLast('\\')
            .map { character -> if (character.code < 32) '_' else character }
            .joinToString("")
            .take(160)
            .ifBlank { "tau-attachment" }
        val context = TauAndroidContext.require()
        val extension = safeName.substringAfterLast('.', "").lowercase()
        val mimeType = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension)
            ?: "application/octet-stream"
        if (Build.VERSION.SDK_INT >= 29) {
            val values = ContentValues().apply {
                put(MediaStore.Downloads.DISPLAY_NAME, safeName)
                put(MediaStore.Downloads.MIME_TYPE, mimeType)
                put(
                    MediaStore.Downloads.RELATIVE_PATH,
                    Environment.DIRECTORY_DOWNLOADS + "/Tau",
                )
                put(MediaStore.Downloads.IS_PENDING, 1)
            }
            val resolver = context.contentResolver
            val uri = checkNotNull(
                resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values),
            ) { "Android could not create the download" }
            try {
                checkNotNull(resolver.openOutputStream(uri)).use { it.write(bytes) }
                values.clear()
                values.put(MediaStore.Downloads.IS_PENDING, 0)
                resolver.update(uri, values, null, null)
            } catch (error: Throwable) {
                resolver.delete(uri, null, null)
                throw error
            }
            return SavedDownload(
                location = "Downloads/Tau/$safeName",
                reference = uri.toString(),
                mimeType = mimeType,
            )
        }
        val directory = File(
            checkNotNull(context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS)),
            "Tau",
        )
        directory.mkdirs()
        val target = File(directory, safeName)
        target.writeBytes(bytes)
        val uri = FileProvider.getUriForFile(
            context,
            "${context.packageName}.files",
            target,
        )
        return SavedDownload(
            location = target.absolutePath,
            reference = uri.toString(),
            mimeType = mimeType,
        )
    }

    actual fun openDownload(download: SavedDownload) {
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(Uri.parse(download.reference), download.mimeType)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        TauAndroidContext.require().startActivity(intent)
    }

    actual fun showDownload(download: SavedDownload) {
        error("Showing downloaded files is supported only on Windows.")
    }

    actual fun extractAndOpenDownload(download: SavedDownload) {
        error("Extracting downloaded ZIP files is supported only on Windows.")
    }
}

actual fun Modifier.onSecondaryClick(onClick: (Offset) -> Unit): Modifier = this

@Composable
actual fun Modifier.onFilesDropped(
    enabled: Boolean,
    onDraggingChanged: (Boolean) -> Unit,
    onDrop: (List<String>) -> Unit,
): Modifier = this

@Composable
actual fun Modifier.onClipboardImagePaste(
    enabled: Boolean,
    onPaste: (suspend () -> PickedFile) -> Unit,
): Modifier = this

actual fun Modifier.onInterruptShortcut(enabled: Boolean, onInterrupt: () -> Unit): Modifier = this

@Composable
actual fun Modifier.onTranscriptAutoscroll(state: LazyListState): Modifier = this

@Composable
actual fun rememberTranscriptScrollMotion() = TranscriptScrollMotion(
    modifier = Modifier,
    flingBehavior = ScrollableDefaults.flingBehavior(),
)

@Composable
actual fun TranscriptScrollbar(
    state: LazyListState,
    geometry: TranscriptGeometry,
    modifier: Modifier,
) = Unit

@Composable
actual fun PlatformBackHandler(enabled: Boolean, onBack: () -> Unit) {
    BackHandler(enabled, onBack)
}

actual fun platformHttpEngine(): HttpClientEngine = OkHttp.create()
