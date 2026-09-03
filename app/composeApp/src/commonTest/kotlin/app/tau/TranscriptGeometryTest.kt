package app.tau

import kotlin.test.Test
import kotlin.test.assertEquals

class TranscriptGeometryTest {
    @Test
    fun mapsExactPixelOffsetsInBothDirections() {
        val geometry = TranscriptGeometry(
            itemHeights = listOf(10, 20, 30),
            itemSpacing = 2,
            beforeContentPadding = 3,
            afterContentPadding = 5,
            viewportSize = 25,
        )

        assertEquals(72.0, geometry.contentSize)
        assertEquals(47.0, geometry.maxScrollOffset)
        assertEquals(0.0, geometry.scrollOffset(0, 0))
        assertEquals(11.0, geometry.scrollOffset(0, 11))
        assertEquals(12.0, geometry.scrollOffset(1, 0))
        assertEquals(37.0, geometry.scrollOffset(2, 3))
        assertEquals(TranscriptPosition(0, 11), geometry.positionAt(11.0))
        assertEquals(TranscriptPosition(1, 0), geometry.positionAt(12.0))
        assertEquals(TranscriptPosition(2, 13), geometry.positionAt(47.0))
    }

    @Test
    fun clampsShortAndEmptyTranscriptsToTheViewport() {
        val short = TranscriptGeometry(
            itemHeights = listOf(10),
            itemSpacing = 12,
            beforeContentPadding = 4,
            afterContentPadding = 4,
            viewportSize = 100,
        )
        val empty = TranscriptGeometry(
            itemHeights = emptyList(),
            itemSpacing = 12,
            beforeContentPadding = 4,
            afterContentPadding = 4,
            viewportSize = 100,
        )

        assertEquals(100.0, short.contentSize)
        assertEquals(0.0, short.maxScrollOffset)
        assertEquals(0.0, short.scrollOffset(0, 20))
        assertEquals(TranscriptPosition(0, 0), empty.positionAt(50.0))
    }
}
