package app.tau

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class TranscriptFlingTest {
    @Test
    fun usesFiniteConstantDeceleration() {
        val duration = transcriptTouchpadFlingDurationMillis(
            velocity = 4_200f,
            maximumVelocity = 4_200f,
            deceleration = 3_000f,
        )

        assertEquals(1_400, duration)
        assertEquals(2_940f, transcriptTouchpadFlingDistance(4_200f, duration))
        assertEquals(
            0.04f,
            transcriptTouchpadFlingDistance(
                velocity = 1f,
                durationMillis = transcriptTouchpadFlingDurationMillis(1f, 4_200f, 3_000f),
            ),
        )
        assertEquals(0f, transcriptTouchpadFlingProgress(0f))
        assertEquals(0.75f, transcriptTouchpadFlingProgress(0.5f))
        assertEquals(1f, transcriptTouchpadFlingProgress(1f))
        assertTrue(
            transcriptTouchpadFlingProgress(1f) - transcriptTouchpadFlingProgress(0.9f) <
                transcriptTouchpadFlingProgress(0.1f) - transcriptTouchpadFlingProgress(0f),
        )
    }
}
