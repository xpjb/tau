package app.tau

import kotlin.math.max

data class TranscriptPosition(
    val index: Int,
    val scrollOffset: Int,
)

class TranscriptGeometry(
    itemHeights: List<Int>,
    private val itemSpacing: Int,
    beforeContentPadding: Int,
    afterContentPadding: Int,
    val viewportSize: Int,
) {
    private val heights = itemHeights.map { it.coerceAtLeast(0) }
    private val starts = LongArray(heights.size)

    val contentSize: Double
    val maxScrollOffset: Double

    init {
        var position = 0L
        heights.indices.forEach { index ->
            starts[index] = position
            position += heights[index]
            if (index != heights.lastIndex) position += itemSpacing
        }
        val measured = position + beforeContentPadding + afterContentPadding
        contentSize = max(measured, viewportSize.toLong()).toDouble()
        maxScrollOffset = (contentSize - viewportSize).coerceAtLeast(0.0)
    }

    fun scrollOffset(index: Int, offset: Int): Double {
        if (starts.isEmpty()) return 0.0
        val safeIndex = index.coerceIn(starts.indices)
        return (starts[safeIndex] + offset.coerceAtLeast(0))
            .toDouble()
            .coerceIn(0.0, maxScrollOffset)
    }

    fun itemEnd(index: Int): Double {
        if (starts.isEmpty()) return 0.0
        val safeIndex = index.coerceIn(starts.indices)
        return (starts[safeIndex] + heights[safeIndex]).toDouble()
    }

    fun positionAt(scrollOffset: Double): TranscriptPosition {
        if (starts.isEmpty()) return TranscriptPosition(0, 0)
        val target = scrollOffset.coerceIn(0.0, maxScrollOffset).toLong()
        var low = 0
        var high = starts.lastIndex
        while (low < high) {
            val middle = (low + high + 1) / 2
            if (starts[middle] <= target) {
                low = middle
            } else {
                high = middle - 1
            }
        }
        val index = low
        return TranscriptPosition(index, (target - starts[index]).toInt())
    }
}
