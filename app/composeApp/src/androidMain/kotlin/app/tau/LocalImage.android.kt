package app.tau

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.graphics.Matrix
import android.media.ExifInterface
import android.os.Build
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import java.io.File
import kotlin.math.roundToInt

@Suppress("DEPRECATION")
internal actual fun decodeLocalImage(path: String, maxSide: Int): ImageBitmap {
    require(maxSide in 1..4096)
    val file = File(path)
    check(file.length() in 1..10_000_000L) { "Image file exceeds Tau's limit" }
    if (Build.VERSION.SDK_INT >= 28) {
        return ImageDecoder.decodeBitmap(ImageDecoder.createSource(file)) { decoder, info, _ ->
            val width = info.size.width
            val height = info.size.height
            check(width in 1..12_000 && height in 1..12_000 && width.toLong() * height <= 64_000_000) {
                "Image dimensions exceed Tau's limit"
            }
            val scale = minOf(1f, maxSide.toFloat() / maxOf(width, height))
            decoder.setTargetSize((width * scale).roundToInt().coerceAtLeast(1), (height * scale).roundToInt().coerceAtLeast(1))
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            decoder.setOnPartialImageListener { false }
        }.asImageBitmap()
    }
    val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(path, options)
    check(options.outWidth in 1..12_000 && options.outHeight in 1..12_000 &&
        options.outWidth.toLong() * options.outHeight <= 64_000_000) { "Image dimensions exceed Tau's limit" }
    options.inJustDecodeBounds = false
    options.inSampleSize = 1
    options.inPreferredConfig = Bitmap.Config.ARGB_8888
    while (options.outWidth / options.inSampleSize > maxSide || options.outHeight / options.inSampleSize > maxSide) options.inSampleSize *= 2
    val bitmap = checkNotNull(BitmapFactory.decodeFile(path, options)) { "Image could not be decoded" }
    val orientation = runCatching { ExifInterface(path).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL) }
        .getOrDefault(ExifInterface.ORIENTATION_NORMAL)
    val matrix = Matrix()
    when (orientation) {
        ExifInterface.ORIENTATION_FLIP_HORIZONTAL -> matrix.setScale(-1f, 1f)
        ExifInterface.ORIENTATION_ROTATE_180 -> matrix.setRotate(180f)
        ExifInterface.ORIENTATION_FLIP_VERTICAL -> matrix.setScale(1f, -1f)
        ExifInterface.ORIENTATION_TRANSPOSE -> { matrix.setRotate(90f); matrix.postScale(-1f, 1f) }
        ExifInterface.ORIENTATION_ROTATE_90 -> matrix.setRotate(90f)
        ExifInterface.ORIENTATION_TRANSVERSE -> { matrix.setRotate(270f); matrix.postScale(-1f, 1f) }
        ExifInterface.ORIENTATION_ROTATE_270 -> matrix.setRotate(270f)
    }
    if (matrix.isIdentity) return bitmap.asImageBitmap()
    val oriented = Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, matrix, true)
    if (oriented !== bitmap) bitmap.recycle()
    return oriented.asImageBitmap()
}
