package app.tau

import androidx.compose.foundation.Image
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import kotlin.math.pow

private val imageDecodeGate = Semaphore(1)

internal data class ImageZoom(val scale: Float = 1f, val offset: Offset = Offset.Zero) {
    fun change(viewport: Size, image: Size, anchor: Offset, factor: Float = 1f, pan: Offset = Offset.Zero): ImageZoom {
        val next = (scale * factor).coerceIn(1f, 8f)
        if (next == 1f) return ImageZoom()
        val point = anchor - Offset(viewport.width / 2, viewport.height / 2)
        val moved = (offset - point) * (next / scale) + point + pan
        val fit = minOf(viewport.width / image.width, viewport.height / image.height)
        val horizontal = ((image.width * fit * next - viewport.width) / 2).coerceAtLeast(0f)
        val vertical = ((image.height * fit * next - viewport.height) / 2).coerceAtLeast(0f)
        return ImageZoom(next, Offset(moved.x.coerceIn(-horizontal, horizontal), moved.y.coerceIn(-vertical, vertical)))
    }
}

@Composable
private fun ZoomableImage(bitmap: ImageBitmap, label: String) {
    BoxWithConstraints(Modifier.fillMaxSize().clipToBounds()) {
        val viewport = Size(constraints.maxWidth.toFloat(), constraints.maxHeight.toFloat())
        val image = Size(bitmap.width.toFloat(), bitmap.height.toFloat())
        val center = Offset(viewport.width / 2, viewport.height / 2)
        var zoom by remember(bitmap, viewport) { mutableStateOf(ImageZoom()) }
        Image(bitmap, label, Modifier.fillMaxSize()
            .pointerInput(bitmap, viewport) {
                detectTransformGestures { anchor, pan, factor, _ ->
                    zoom = zoom.change(viewport, image, anchor, factor, pan)
                }
            }
            .pointerInput(bitmap, viewport) {
                awaitPointerEventScope {
                    while (true) {
                        val event = awaitPointerEvent()
                        if (event.type == PointerEventType.Scroll) {
                            for (change in event.changes) if (!change.isConsumed && change.scrollDelta.y != 0f) {
                                zoom = zoom.change(viewport, image, change.position, 2f.pow(-change.scrollDelta.y / 4f))
                                change.consume()
                            }
                        }
                    }
                }
            }
            .graphicsLayer {
                scaleX = zoom.scale; scaleY = zoom.scale
                translationX = zoom.offset.x; translationY = zoom.offset.y
            }, contentScale = ContentScale.Fit)
        Surface(Modifier.align(Alignment.BottomCenter).padding(8.dp), shape = MaterialTheme.shapes.small) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = { zoom = zoom.change(viewport, image, center, 0.5f) }, enabled = zoom.scale > 1f,
                    modifier = Modifier.semantics { contentDescription = "Zoom out" }) { Text("−") }
                TextButton(onClick = { zoom = ImageZoom() }, enabled = zoom.scale > 1f,
                    modifier = Modifier.semantics { contentDescription = "Fit image" }) { Text("Fit") }
                TextButton(onClick = { zoom = zoom.change(viewport, image, center, 2f) }, enabled = zoom.scale < 8f,
                    modifier = Modifier.semantics { contentDescription = "Zoom in" }) { Text("+") }
            }
        }
    }
}

@Composable
internal fun LocalImage(
    download: AttachmentDownload?,
    label: String,
    visible: Boolean,
    maxSide: Int,
    modifier: Modifier,
    onRetry: () -> Unit,
    zoomable: Boolean = false,
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
            bitmap != null -> if (zoomable) ZoomableImage(bitmap, label)
                else Image(bitmap, label, Modifier.fillMaxSize(), contentScale = ContentScale.Fit)
            error != null -> Column(Modifier.padding(12.dp), horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(error, style = MaterialTheme.typography.labelMedium)
                TextButton(onClick = onRetry) { Text("Retry") }
            }
            visible -> CircularProgressIndicator(Modifier.size(28.dp), strokeWidth = 2.dp)
        }
    }
}
