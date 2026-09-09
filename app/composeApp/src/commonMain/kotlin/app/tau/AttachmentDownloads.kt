package app.tau

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.flow.update
import kotlin.time.TimeSource

internal fun TauController.downloadAttachment(sessionId: String, message: TranscriptEvent, action: AttachmentDownloadAction) {
    val current = state.value
    if (sessionId != current.selectedSessionId) return
    val attachment = message.attachment ?: return
    val key = AttachmentDownloadKey(sessionId, message.entryId)
    if (key in downloadJobs) return
    val previous = current.attachmentDownloads[key]
    val settings = current.settings
    val allowNetwork = current.connectionStatus == ConnectionStatus.Connected
    if (action == AttachmentDownloadAction.Preview && previous != null &&
        !(previous.failure == AttachmentFailure.NotLocal && allowNetwork)) return
    val attempt = (previous?.attempt ?: 0) + 1
    mutableState.update {
        it.copy(attachmentDownloads = it.attachmentDownloads + (key to AttachmentDownload(
            status = AttachmentDownloadStatus.Downloading,
            transferredBytes = 0,
            totalBytes = attachment.size,
            localPath = if (action == AttachmentDownloadAction.Reload) null else previous?.localPath,
            saved = previous?.saved,
            attempt = attempt,
        )), error = null)
    }
    lateinit var job: Job
    job = scope.launch(start = CoroutineStart.LAZY) {
        val started = TimeSource.Monotonic.markNow()
        var transferredBytes = 0L
        var totalBytes = attachment.size
        var lastPublishedBytes = 0L
        var lastPublishedMillis = 0L
        try {
            val path = client.downloadAttachment(
                settings, sessionId, message.entryId,
                if (attachment.kind == AttachmentKind.Image) 10_000_000L else 50_000_000L,
                force = action == AttachmentDownloadAction.Reload, allowNetwork = allowNetwork,
            ) { transferred, total ->
                transferredBytes = transferred
                if (total != null) totalBytes = total
                val elapsedMillis = started.elapsedNow().inWholeMilliseconds.coerceAtLeast(1)
                if (elapsedMillis - lastPublishedMillis >= DownloadProgressIntervalMillis || totalBytes?.let { transferred >= it } == true) {
                    val intervalMillis = (elapsedMillis - lastPublishedMillis).coerceAtLeast(1)
                    val bytesPerSecond = (transferred - lastPublishedBytes).coerceAtLeast(0) * 1_000 / intervalMillis
                    lastPublishedBytes = transferred
                    lastPublishedMillis = elapsedMillis
                    withContext(dispatcher) { mutableState.update { ui ->
                        val active = ui.attachmentDownloads[key]
                        if (downloadJobs[key] !== job || active?.status != AttachmentDownloadStatus.Downloading) ui
                        else ui.copy(attachmentDownloads = ui.attachmentDownloads + (key to active.copy(
                            transferredBytes = transferred, totalBytes = totalBytes, bytesPerSecond = bytesPerSecond,
                        )))
                    } }
                }
            }
            val download = if (action == AttachmentDownloadAction.Save) withContext(Dispatchers.IO) {
                PlatformServices.saveDownload(attachment.fileName, path)
            } else previous?.saved
            mutableState.update { ui ->
                if (downloadJobs[key] !== job) ui
                else ui.copy(
                    attachmentDownloads = ui.attachmentDownloads + (key to AttachmentDownload(
                        status = AttachmentDownloadStatus.Downloaded,
                        transferredBytes = transferredBytes,
                        totalBytes = totalBytes ?: transferredBytes,
                        saved = download,
                        localPath = path,
                        attempt = attempt,
                    )),
                    notice = if (action == AttachmentDownloadAction.Save && download != null) "Saved to ${download.location}" else ui.notice,
                    error = null,
                )
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Throwable) {
            val failure = (error as? AttachmentDownloadException)?.failure ?: AttachmentFailure.Interrupted
            mutableState.update { ui ->
                val active = ui.attachmentDownloads[key]
                if (downloadJobs[key] !== job || active == null) ui
                else ui.copy(attachmentDownloads = ui.attachmentDownloads + (key to active.copy(
                    status = AttachmentDownloadStatus.Failed, bytesPerSecond = null, failure = failure,
                )))
            }
        } finally {
            if (downloadJobs[key] === job) downloadJobs.remove(key)
        }
    }
    downloadJobs[key] = job
    job.start()
}

internal fun TauController.cancelAttachmentDownload(message: TranscriptEvent) {
    val sessionId = mutableState.value.selectedSessionId ?: return
    val key = AttachmentDownloadKey(sessionId, message.entryId)
    downloadJobs.remove(key)?.cancel()
    mutableState.update {
        val active = it.attachmentDownloads[key]
        if (active == null) it else it.copy(attachmentDownloads = it.attachmentDownloads +
            (key to active.copy(status = AttachmentDownloadStatus.Failed, bytesPerSecond = null, failure = AttachmentFailure.Cancelled)))
    }
}

internal fun TauController.openAttachmentDownload(message: TranscriptEvent) {
    val sessionId = mutableState.value.selectedSessionId ?: return
    val key = AttachmentDownloadKey(sessionId, message.entryId)
    val download = mutableState.value.attachmentDownloads[key]?.saved ?: return
    try {
        PlatformServices.openDownload(download)
    } catch (error: Throwable) {
        mutableState.update {
            it.copy(error = error.message ?: "The downloaded file could not be opened.")
        }
    }
}

internal fun TauController.showAttachmentDownload(message: TranscriptEvent) {
    val sessionId = mutableState.value.selectedSessionId ?: return
    val key = AttachmentDownloadKey(sessionId, message.entryId)
    val download = mutableState.value.attachmentDownloads[key]?.saved ?: return
    try {
        PlatformServices.showDownload(download)
    } catch (error: Throwable) {
        mutableState.update {
            it.copy(error = error.message ?: "The downloaded file could not be shown.")
        }
    }
}

internal fun TauController.extractAndOpenAttachmentDownload(message: TranscriptEvent) {
    val sessionId = mutableState.value.selectedSessionId ?: return
    val key = AttachmentDownloadKey(sessionId, message.entryId)
    val download = mutableState.value.attachmentDownloads[key]?.saved ?: return
    try {
        PlatformServices.extractAndOpenDownload(download)
    } catch (error: Throwable) {
        mutableState.update {
            it.copy(error = error.message ?: "The downloaded ZIP could not be extracted.")
        }
    }
}
