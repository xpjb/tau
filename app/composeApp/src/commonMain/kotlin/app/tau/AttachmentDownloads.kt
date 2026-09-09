package app.tau

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
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
        !(previous.status == AttachmentDownloadStatus.Downloaded && previous.localPath == null) &&
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
            val download = if (action == AttachmentDownloadAction.Save) withContext(NonCancellable) {
                val saved = withContext(Dispatchers.IO) {
                    previous?.saved?.takeIf(PlatformServices::downloadExists) ?: PlatformServices.saveDownload(attachment.fileName, path)
                }
                store.recordDownload(ChatKey(settings.identity, sessionId), message.entryId, saved)
                saved
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

internal fun TauController.useAttachmentDownload(message: TranscriptEvent, action: (SavedDownload) -> Unit) {
    val current = state.value
    val sessionId = current.selectedSessionId ?: return
    val key = AttachmentDownloadKey(sessionId, message.entryId)
    val download = current.attachmentDownloads[key]?.saved ?: return
    scope.launch {
        try {
            val exists = withContext(Dispatchers.IO) { PlatformServices.downloadExists(download) }
            if (!exists) store.recordDownload(ChatKey(current.settings.identity, sessionId), message.entryId, download, available = false)
            if (state.value.settings.identity != current.settings.identity) return@launch
            if (!exists) {
                mutableState.update { ui ->
                    val stored = ui.attachmentDownloads[key]
                    if (stored?.saved != download) ui
                    else ui.copy(attachmentDownloads = ui.attachmentDownloads + (key to stored.copy(saved = null)))
                }
                error("The downloaded file is no longer available. Save it again.")
            }
            action(download)
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Throwable) {
            if (state.value.settings.identity == current.settings.identity) mutableState.update {
                it.copy(error = error.message ?: "The downloaded file could not be opened.")
            }
        }
    }
}
