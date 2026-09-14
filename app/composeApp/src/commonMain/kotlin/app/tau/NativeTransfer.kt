package app.tau

internal data class TransferProgress(val transferred: Long, val total: Long, val done: Boolean, val failure: String?)

internal interface NativeTransfer {
    val nodeId: String
    fun start(offer: String, host: String, target: String, limit: Long)
    fun progress(): TransferProgress
    fun close()
}

internal expect fun platformTransfer(): NativeTransfer
