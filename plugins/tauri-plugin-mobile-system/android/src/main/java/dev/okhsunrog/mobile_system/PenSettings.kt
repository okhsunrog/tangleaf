package dev.okhsunrog.mobile_system

/** One firmware setter of the pen configuration, in the order it is sent. */
internal sealed interface PenCall {
    /** `Device.setStrokeParameters`: per-painter parameters, kept by SurfaceFlinger per style. */
    data class StrokeParameters(val style: Int, val values: List<Float>) : PenCall
    data class StrokeWidth(val width: Float) : PenCall
    data class StrokeColor(val color: Int) : PenCall

    /** The single style slot the tip draws with; the eraser end overwrites it. */
    data class StrokeStyle(val style: Int) : PenCall

    /** Whether SurfaceFlinger draws the eraser end, and with which painter. */
    data class EraserTrace(val enabled: Boolean, val style: Int) : PenCall
    data class BrushTrace(val enabled: Boolean) : PenCall
}

/** SurfaceFlinger's painters, as `TouchHelper` and stock's resume name them. */
internal object PenStyle {
    const val FOUNTAIN = 1

    /** The dash: the lasso trace, and stock's lasso eraser. */
    const val DASH = 5

    /** `PainterSoftEraser`, stock's trace for the stroke and pixel erasers. */
    const val SOFT_ERASER = 8
}

/** Dash length and gap, and the line width, of the firmware selection trace. */
internal const val LASSO_DASH = 5f

/** `Color.BLACK`, spelled out so this file stays free of the Android framework. */
internal const val INK_BLACK = -0x1000000

/** Stock's soft-eraser opacities (`ERASE_PEN_OPACITY`, `ERASE_PEN_BLACK_OPACITY`). */
private const val ERASER_OPACITY = 0.5f
private const val ERASER_BLACK_OPACITY = 0.1f

/** The eraser menu's lasso type, stock's area eraser: its trace is the dash, not the soft eraser. */
private const val LASSO_TYPE = "lasso"

/**
 * Whether the firmware draws an erase gesture's trace, as stock does, and the erase hand-back takes
 * it away after the lift ([InkEraseHandBack]): the eraser end with the pen tool, and the eraser
 * tool's tip and eraser end, for every eraser type. The soft eraser draws the stroke and pixel
 * types, the dash the lasso type ([eraserEndStyle]). Only the lasso tool erases with render
 * paused: its eraser end has no firmware trace, and the page previews that erase itself.
 *
 * @param erasing whether the SDK reports the gesture as the eraser end.
 * @param eraser whether the eraser tool is active: its tip erases as well.
 */
internal fun eraseTrace(erasing: Boolean, eraser: Boolean, interaction: Boolean): Boolean =
    (erasing || eraser) && !interaction

/**
 * Whether the active tool wants firmware ink rather than only its raw points. The eraser tool
 * renders as stock's does, the trace of its main style, whatever its type. The lasso tool renders
 * only its free-form selection trace, and only while nothing is selected.
 */
internal fun toolRenders(
    eraser: Boolean,
    interaction: Boolean,
    fastLasso: Boolean,
    hasSelection: Boolean,
): Boolean = eraser || !interaction || (fastLasso && !hasSelection)

/** The painter the tip draws with: the tool's main style, as the resume sends it. */
internal fun toolStrokeStyle(eraser: Boolean, fastLasso: Boolean, eraserType: String): Int = when {
    fastLasso -> PenStyle.DASH
    eraser -> eraserEndStyle(eraserType)
    else -> PenStyle.FOUNTAIN
}

/**
 * The painter the eraser draws with: the eraser end writes it into the style slot on touch-down,
 * and the eraser tool makes it its main style. Stock's `ResumeRawDrawingRequest`: style 8 for its
 * stroke and move erasers, style 5, the dash, for its area eraser.
 */
internal fun eraserEndStyle(eraserType: String): Int =
    if (eraserType == LASSO_TYPE) PenStyle.DASH else PenStyle.SOFT_ERASER

/**
 * Whether an eraser-end contact with render on leaves the style slot holding something other than
 * the tool's own style. With the eraser tool both are the eraser's own painter, so its tip needs no
 * re-send; with the pen they differ for every type.
 */
internal fun eraserEndOverwritesStyle(eraser: Boolean, fastLasso: Boolean, eraserType: String): Boolean =
    toolStrokeStyle(eraser, fastLasso, eraserType) != eraserEndStyle(eraserType)

/**
 * Whether the SDK's pen-up refresh may start a reconcile. The eraser tool's tip strokes are ordinary
 * strokes to the SDK, so its pen-up timer fires about 500 ms after each lift; the erase hand-back
 * owns those gestures, and a reconcile would put a repaint-mode frame inside it. Stock's consumer
 * for its eraser tool does nothing either.
 */
internal fun penUpRefreshReconciles(eraser: Boolean, interaction: Boolean): Boolean =
    !(eraser && eraseTrace(erasing = false, eraser = true, interaction = interaction))

/**
 * The full pen configuration pushed on every resume, stock's `ResumeRawDrawingRequest` set.
 *
 * The dash parameters go first: the firmware draws the selection trace as a dashed line only when
 * the dash pattern is configured on the device before the style, a single-element gap/length
 * array, black, at the driver's standard width (Notate's verified recipe, onyx.md §2). Then the
 * soft eraser's width, so the eraser's trace is as wide as the eraser; then the tool's own
 * width, colour and style in that verified order. Stock sends colour, style, width and the
 * parameters after them; each is its own firmware setter.
 *
 * The eraser tool makes its type's painter its main style and sends no width, as stock does on
 * selecting its eraser (`SET_STROKE_STYLE [8]`, or `[5]` for the area eraser, and no
 * `SET_STROKE_WIDTH`): the soft eraser takes its width from its own parameters, and the dash is
 * drawn at whatever width the previous tool left.
 *
 * The eraser end is drawn by the firmware for every type, with the type's painter. It is off while
 * the lasso tool is active, which erases with render paused: SurfaceFlinger then picks the reader
 * that paints nothing and leaves the style slot alone, so the next lasso stroke stays dashed.
 */
internal fun penSettingsCalls(
    fastLasso: Boolean,
    interaction: Boolean,
    eraser: Boolean,
    eraserType: String,
    eraserPx: Float,
    penPx: Float,
): List<PenCall> {
    val style = toolStrokeStyle(eraser, fastLasso, eraserType)
    return buildList {
        add(PenCall.StrokeParameters(PenStyle.DASH, listOf(LASSO_DASH)))
        add(PenCall.StrokeParameters(PenStyle.SOFT_ERASER, listOf(eraserPx, ERASER_OPACITY, ERASER_BLACK_OPACITY)))
        when {
            eraser -> Unit
            fastLasso -> add(PenCall.StrokeWidth(LASSO_DASH))
            else -> add(PenCall.StrokeWidth(penPx))
        }
        add(PenCall.StrokeColor(INK_BLACK))
        add(PenCall.StrokeStyle(style))
        add(PenCall.EraserTrace(!interaction, eraserEndStyle(eraserType)))
        add(PenCall.BrushTrace(true))
    }
}
