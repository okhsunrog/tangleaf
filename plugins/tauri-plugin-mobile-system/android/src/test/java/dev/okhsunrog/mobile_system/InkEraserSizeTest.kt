package dev.okhsunrog.mobile_system

import org.junit.Assert.*
import org.junit.Test

class InkEraserSizeTest {
    @Test fun widthsMatchStockBitForBitAt300Dpi() {
        // The payloads of SET_STROKE_PARAMETERS(8, …) recorded from stock Notes.
        assertEquals(1114388593, java.lang.Float.floatToIntBits(eraserPx(500, 300))) // 5.00 mm
        assertEquals(1086126580, java.lang.Float.floatToIntBits(eraserPx(50, 300))) // 0.50 mm
        assertEquals(1117823299, java.lang.Float.floatToIntBits(eraserPx(680, 300))) // 6.80 mm
    }

    @Test fun theRadiusIsHalfTheWidthInSheetUnits() {
        // A 1655 px sheet a thousand units across: 59.05512 px is 35.68 units, half of it 17.84.
        assertEquals(17.841, eraserRadiusUnits(59.05512f, 1655f, 1000.0), 0.001)
        assertEquals(5.0, eraserRadiusUnits(20f, 2000f, 1000.0), 0.0)
    }

    @Test fun theMenuRangeCoversEveryType() {
        assertEquals(setOf("stroke", "pixel", "lasso"), InkEraserSize.TYPES)
        assertTrue(50 in InkEraserSize.MIN_DIAMETER..InkEraserSize.MAX_DIAMETER)
        assertFalse(801 in InkEraserSize.MIN_DIAMETER..InkEraserSize.MAX_DIAMETER)
    }
}
