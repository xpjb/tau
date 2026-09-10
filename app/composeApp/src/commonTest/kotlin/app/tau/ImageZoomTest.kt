package app.tau

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import kotlin.math.abs
import kotlin.random.Random
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class ImageZoomTest {
    @Test
    fun keepsTheAnchorAndBoundsPanAcrossAspectRatios() {
        val random = Random(42)
        for (viewport in listOf(Size(800f, 600f), Size(360f, 720f), Size(1f, 1f))) {
            val center = Offset(viewport.width / 2, viewport.height / 2)
            for (image in listOf(Size(1600f, 1200f), Size(400f, 2400f), Size(2400f, 400f), Size(1f, 1f))) {
                val fit = minOf(viewport.width / image.width, viewport.height / image.height)
                var zoom = ImageZoom()
                repeat(100) {
                    val anchor = Offset(random.nextFloat() * viewport.width, random.nextFloat() * viewport.height)
                    val pan = Offset((random.nextFloat() - 0.5f) * 40, (random.nextFloat() - 0.5f) * 40)
                    val point = (anchor - center - zoom.offset) / zoom.scale
                    val next = zoom.change(viewport, image, anchor, 0.5f + random.nextFloat() * 2, pan)
                    assertTrue(next.scale in 1f..8f)
                    val horizontal = ((image.width * fit * next.scale - viewport.width) / 2).coerceAtLeast(0f)
                    val vertical = ((image.height * fit * next.scale - viewport.height) / 2).coerceAtLeast(0f)
                    assertTrue(abs(next.offset.x) <= horizontal && abs(next.offset.y) <= vertical)
                    val position = center + point * next.scale + next.offset
                    if (abs(next.offset.x) < horizontal - 0.001f) assertEquals(anchor.x + pan.x, position.x, 0.002f)
                    if (abs(next.offset.y) < vertical - 0.001f) assertEquals(anchor.y + pan.y, position.y, 0.002f)
                    zoom = next
                }
                val maximum = zoom.change(viewport, image, center, 100f, Offset(100_000f, -100_000f))
                assertEquals(8f, maximum.scale)
                assertEquals(((image.width * fit * 8 - viewport.width) / 2).coerceAtLeast(0f), maximum.offset.x, 0.001f)
                assertEquals(-((image.height * fit * 8 - viewport.height) / 2).coerceAtLeast(0f), maximum.offset.y, 0.001f)
                val capped = maximum.change(viewport, image, center, 2f)
                assertEquals(maximum.scale, capped.scale)
                assertEquals(maximum.offset.x, capped.offset.x, 0.001f)
                assertEquals(maximum.offset.y, capped.offset.y, 0.001f)
                assertEquals(ImageZoom(), maximum.change(viewport, image, center, 0.01f))
                assertEquals(ImageZoom(), ImageZoom().change(viewport, image, center, pan = Offset(100f, 200f)))
            }
        }
        val viewport = Size(800f, 600f)
        val anchor = Offset(550f, 220f)
        val original = ImageZoom(2f, Offset(20f, -30f))
        val enlarged = original.change(viewport, viewport, anchor, 1.5f, Offset(12f, 18f))
        val restored = enlarged.change(viewport, viewport, anchor + Offset(12f, 18f), 1 / 1.5f, Offset(-12f, -18f))
        assertEquals(original.scale, restored.scale, 0.001f)
        assertEquals(original.offset.x, restored.offset.x, 0.001f)
        assertEquals(original.offset.y, restored.offset.y, 0.001f)
    }
}
