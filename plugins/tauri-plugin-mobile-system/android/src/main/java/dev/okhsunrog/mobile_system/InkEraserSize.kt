package dev.okhsunrog.mobile_system

/**
 * The eraser's size, in the stock Notes app's own units and arithmetic.
 *
 * The page stores a diameter in hundredths of a millimetre, as stock does (`NoteUnits`). Stock turns
 * it into device pixels only when it hands the width to the firmware, `mm * densityDpi / 25.4` in
 * float (`DimensKt.mmToPx`), and uses the same pixels, halved, as the reach of its hit test. Both
 * are reproduced here so the numbers can be compared with stock's recordings bit for bit.
 */
internal object InkEraserSize {
    /** The menu's range over all types (stroke 0.50 mm up to 8.00 mm); the lasso keeps 0.50 mm. */
    const val MIN_DIAMETER = 50
    const val MAX_DIAMETER = 800
    val TYPES = setOf("stroke", "pixel", "lasso")
}

/**
 * Stock's `mmToPx(fromPercentage(hundredths))` at [dpi], float for float: 5.00 mm at 300 dpi is
 * `59.05512f`, the width stock sends for its 5.00 mm pixel eraser.
 */
internal fun eraserPx(hundredths: Int, dpi: Int): Float = (hundredths / 100f) * dpi / 25.4f

/** Half of [px] as a distance on a sheet [unitsWide] units across that is [sheetWidthPx] wide. */
internal fun eraserRadiusUnits(px: Float, sheetWidthPx: Float, unitsWide: Double): Double =
    px / 2.0 / sheetWidthPx * unitsWide
