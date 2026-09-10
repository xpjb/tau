package app.tau

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.InternalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.PointerId
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.scene.ComposeScenePointer
import androidx.compose.ui.unit.Constraints
import java.awt.image.BufferedImage
import java.nio.file.Files
import javax.imageio.ImageIO
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.swing.Swing
import kotlinx.coroutines.withTimeout
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

@OptIn(ExperimentalComposeUiApi::class, InternalComposeUiApi::class)
class ImageGestureTest {
    @Test
    fun pinchesPansAndKeepsZoomAfterRelease() = runBlocking(Dispatchers.Swing) {
        val file = Files.createTempFile("tau-image-gesture-", ".png").toFile()
        val image = BufferedImage(800, 800, BufferedImage.TYPE_INT_RGB)
        image.createGraphics().apply {
            color = java.awt.Color.CYAN; fillRect(0, 0, 800, 800)
            color = java.awt.Color.RED; fillRect(512, 272, 40, 40)
            dispose()
        }
        ImageIO.write(image, "png", file)
        val download = AttachmentDownload(AttachmentDownloadStatus.Downloaded, file.length(), file.length(), localPath = file.path)
        val scene = ImageComposeScene(600, 600, coroutineContext = Dispatchers.Swing) {
            MaterialTheme {
                LocalImage(download, "Touch image", true, 4096, Modifier.fillMaxSize(), onRetry = { error("Unexpected retry") }, zoomable = true)
            }
        }
        suspend fun marker(): Rect = withTimeout(5_000) {
            var found: Rect? = null
            while (found == null) {
                scene.render().use { rendered ->
                    val pixels = rendered.toComposeImageBitmap().toPixelMap()
                    var left = pixels.width; var top = pixels.height; var right = -1; var bottom = -1
                    for (y in 0 until pixels.height) for (x in 0 until pixels.width) {
                        val pixel = pixels[x, y]
                        if (pixel.red > 0.94f && pixel.green < 0.125f && pixel.blue < 0.125f) {
                            left = minOf(left, x); right = maxOf(right, x)
                            top = minOf(top, y); bottom = maxOf(bottom, y)
                        }
                    }
                    if (right >= left && bottom >= top) found = Rect(left.toFloat(), top.toFloat(), (right + 1).toFloat(), (bottom + 1).toFloat())
                }
                if (found == null) delay(20)
            }
            found
        }
        var time = 0L
        fun touch(type: PointerEventType, vararg points: Pair<Offset, Boolean>) {
            time += 16
            scene.sendPointerEvent(type, points.mapIndexed { index, (point, pressed) ->
                ComposeScenePointer(PointerId(index.toLong()), point, pressed, PointerType.Touch)
            }, timeMillis = time)
            scene.render().close()
        }
        try {
            val fitted = marker()
            val left = fitted.center - Offset(40f, 0f)
            val right = fitted.center + Offset(40f, 0f)
            touch(PointerEventType.Press, left to true)
            touch(PointerEventType.Press, left to true, right to true)
            val movedLeft = left + Offset(-20f, 30f)
            val movedRight = right + Offset(60f, 30f)
            touch(PointerEventType.Move, movedLeft to true, movedRight to true)
            val enlarged = marker()
            assertEquals(fitted.width * 2, enlarged.width, 2f)
            assertEquals(fitted.center.x + 20, enlarged.center.x, 2f)
            assertEquals(fitted.center.y + 30, enlarged.center.y, 2f)
            touch(PointerEventType.Release, movedLeft to true, movedRight to false)
            touch(PointerEventType.Move, (movedLeft + Offset(15f, 10f)) to true)
            touch(PointerEventType.Release, (movedLeft + Offset(15f, 10f)) to false)
            val panned = marker()
            assertEquals(enlarged.width, panned.width)
            assertEquals(enlarged.center.x + 15, panned.center.x, 2f)
            assertEquals(enlarged.center.y + 10, panned.center.y, 2f)
            delay(500)
            assertEquals(panned, marker(), "Touch release reset the zoom")
            scene.constraints = Constraints.fixed(400, 600)
            val resized = marker()
            assertEquals(20f, resized.width, 2f)
            assertTrue(abs(resized.center.x - 266f) < 2 && abs(resized.center.y - 246f) < 2,
                "Resizing did not return to a fitted image: $resized")
        } finally {
            scene.close()
            file.delete()
        }
    }
}
