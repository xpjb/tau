package app.tau

import app.tau.transfer.TransferDownload
import app.tau.transfer.TransferException

internal actual fun platformTransfer(): NativeTransfer = object : NativeTransfer {
    private val native = TransferDownload()
    override val nodeId: String get() = native.nodeId()

    override fun start(offer: String, host: String, target: String, limit: Long) {
        require(limit >= 0)
        try {
            native.start(offer, host, target, limit.toULong())
        } catch (error: TransferException.Failed) {
            throw AttachmentDownloadException(
                if (error.reason == "too_large") AttachmentFailure.TooLarge else AttachmentFailure.Interrupted, error,
            )
        }
    }

    override fun progress(): TransferProgress {
        val status = native.status()
        return TransferProgress(status.transferred.toLong(), status.total.toLong(), status.done, status.failure)
    }

    override fun close() {
        try {
            native.cancel()
            native.join()
        } finally {
            native.close()
        }
    }
}
