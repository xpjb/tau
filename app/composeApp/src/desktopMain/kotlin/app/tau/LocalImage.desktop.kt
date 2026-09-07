package app.tau

import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import java.nio.file.Files
import java.nio.file.Path
import org.jetbrains.skia.Bitmap
import org.jetbrains.skia.Codec
import org.jetbrains.skia.Data
import org.jetbrains.skia.Image
import org.jetbrains.skia.Surface
import org.jetbrains.skia.Rect
import org.jetbrains.skia.SamplingMode
import kotlin.math.roundToInt

internal actual fun decodeLocalImage(path: String, maxSide: Int): ImageBitmap {
    require(maxSide in 1..4096)
    val file = Path.of(path)
    check(Files.size(file) in 1..10_000_000L) { "Image file exceeds Tau's limit" }
    return Data.makeFromBytes(Files.readAllBytes(file)).use { data ->
        Codec.makeFromData(data).use { codec ->
            val width = codec.width
            val height = codec.height
            check(width in 1..12_000 && height in 1..12_000 && width.toLong() * height <= 64_000_000) {
                "Image dimensions exceed Tau's limit"
            }
            val origin = codec.encodedOrigin
            val orientedWidth = if (origin.swapsWidthHeight()) height else width
            val orientedHeight = if (origin.swapsWidthHeight()) width else height
            val scale = minOf(1f, maxSide.toFloat() / maxOf(orientedWidth, orientedHeight))
            val outputWidth = (orientedWidth * scale).roundToInt().coerceAtLeast(1)
            val outputHeight = (orientedHeight * scale).roundToInt().coerceAtLeast(1)
            Bitmap().use { bitmap ->
                check(bitmap.allocN32Pixels(width, height)) { "Image pixels could not be allocated" }
                codec.readPixels(bitmap)
                bitmap.setImmutable()
                Image.makeFromBitmap(bitmap).use { image ->
                    Surface.makeRasterN32Premul(outputWidth, outputHeight).use { surface ->
                        surface.canvas.clear(0)
                        surface.canvas.scale(outputWidth.toFloat() / orientedWidth, outputHeight.toFloat() / orientedHeight)
                        surface.canvas.concat(origin.toMatrix(orientedWidth, orientedHeight))
                        val source = Rect.makeWH(width.toFloat(), height.toFloat())
                        surface.canvas.drawImageRect(image, source, source, SamplingMode.LINEAR, null, true)
                        surface.makeImageSnapshot().use { it.toComposeImageBitmap() }
                    }
                }
            }
        }
    }
}
