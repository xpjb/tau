package app.tau

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext

private val imageDecodeGate = Semaphore(1)

@Composable
internal fun LocalImage(
    download: AttachmentDownload?,
    label: String,
    visible: Boolean,
    maxSide: Int,
    modifier: Modifier,
    onRetry: () -> Unit,
) {
    val path = download?.localPath
    val decoded by produceState<Result<ImageBitmap>?>(null, path, download?.attempt, visible, maxSide) {
        value = null
        if (visible && path != null) {
            value = try {
                Result.success(withContext(Dispatchers.IO) { imageDecodeGate.withPermit { decodeLocalImage(path, maxSide) } })
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Throwable) { Result.failure(IllegalStateException("Saved image could not be decoded")) }
        }
    }
    Box(modifier, contentAlignment = Alignment.Center) {
        val bitmap = decoded?.getOrNull()
        val error = if (path == null) download?.failure?.message else decoded?.exceptionOrNull()?.message
        when {
            bitmap != null -> Image(bitmap, label, Modifier.fillMaxSize(), contentScale = ContentScale.Fit)
            error != null -> Column(Modifier.padding(12.dp), horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(error, style = MaterialTheme.typography.labelMedium)
                TextButton(onClick = onRetry) { Text("Retry") }
            }
            visible -> CircularProgressIndicator(Modifier.size(28.dp), strokeWidth = 2.dp)
        }
    }
}
