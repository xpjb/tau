package app.tau

import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.file.Files
import javax.imageio.ImageIO
import org.jetbrains.skia.EncodedImageFormat
import org.jetbrains.skia.Image
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFails
import kotlin.test.assertTrue

class LocalImageTest {
    @Test
    fun decodesLocalFormatsTransparencyOrientationAndBounds() {
        val root = Files.createTempDirectory("tau-image-decoder").toFile()
        try {
            val source = BufferedImage(64, 32, BufferedImage.TYPE_INT_ARGB)
            val colors = intArrayOf(0x80ff0000.toInt(), 0xff00ff00.toInt(), 0xffffff00.toInt(), 0xff0000ff.toInt())
            for (y in 0 until 32) for (x in 0 until 64) source.setRGB(x, y, colors[(y / 16) * 2 + x / 32])
            val png = ByteArrayOutputStream().also { ImageIO.write(source, "png", it) }.toByteArray()
            val webp = Image.makeFromEncoded(png).use { image ->
                checkNotNull(image.encodeToData(EncodedImageFormat.WEBP, 100)).use { it.bytes }
            }
            for ((format, bytes) in listOf("png" to png, "webp" to webp)) {
                val path = root.resolve("image.$format").also { it.writeBytes(bytes) }
                val image = decodeLocalImage(path.path, 1024)
                assertEquals(64, image.width)
                assertEquals(32, image.height)
                val pixels = IntArray(image.width * image.height)
                image.readPixels(pixels)
                assertTrue((pixels[8 * 64 + 16] ushr 24) in 126..130)
                assertTrue((pixels[8 * 64 + 48] ushr 8 and 255) >= 240)
                val small = decodeLocalImage(path.path, 16)
                assertEquals(16, small.width)
                assertEquals(8, small.height)
            }
            val rgb = BufferedImage(64, 32, BufferedImage.TYPE_INT_RGB)
            for (y in 0 until 32) for (x in 0 until 64) rgb.setRGB(x, y, colors[(y / 16) * 2 + x / 32] or 0xff000000.toInt())
            val jpeg = ByteArrayOutputStream().also { ImageIO.write(rgb, "jpeg", it) }.toByteArray()
            val corners = intArrayOf(0, 1, 3, 2, 0, 2, 3, 1)
            for (orientation in 1..8) {
                val exif = ByteBuffer.allocate(32).order(ByteOrder.LITTLE_ENDIAN)
                    .put("Exif\u0000\u0000II".toByteArray()).putShort(42).putInt(8).putShort(1)
                    .putShort(0x112).putShort(3).putInt(1).putShort(orientation.toShort()).putShort(0).putInt(0).array()
                val path = root.resolve("oriented.jpg")
                path.writeBytes(jpeg.take(2).toByteArray() + byteArrayOf(0xff.toByte(), 0xe1.toByte(), 0, 34) + exif + jpeg.drop(2).toByteArray())
                val image = decodeLocalImage(path.path, 1024)
                assertEquals(if (orientation < 5) 64 else 32, image.width)
                assertEquals(if (orientation < 5) 32 else 64, image.height)
                val pixels = IntArray(image.width * image.height)
                image.readPixels(pixels)
                val actual = pixels[image.height / 4 * image.width + image.width / 4]
                val expected = colors[corners[orientation - 1]]
                for (shift in listOf(0, 8, 16)) assertTrue(kotlin.math.abs((actual ushr shift and 255) - (expected ushr shift and 255)) <= 20,
                    "Orientation $orientation changed its top-left color")
            }
            val invalid = root.resolve("invalid.png").also { it.writeBytes(png.take(20).toByteArray()) }
            assertFails { decodeLocalImage(invalid.path, 1024) }
            val wide = root.resolve("wide.png")
            ImageIO.write(BufferedImage(12_001, 1, BufferedImage.TYPE_INT_RGB), "png", wide)
            assertFails { decodeLocalImage(wide.path, 1024) }
            assertFails { decodeLocalImage(root.resolve("missing.png").path, 1024) }
        } finally { root.deleteRecursively() }
    }
}
