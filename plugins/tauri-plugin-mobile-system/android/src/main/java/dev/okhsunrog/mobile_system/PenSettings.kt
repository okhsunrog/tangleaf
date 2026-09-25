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

/** The eraser menu's lasso type, which keeps its own path until the firmware draws its dash trace. */
private const val LASSO_TYPE = "lasso"

/**
 * Whether the firmware draws an erase gesture's trace, as stock does, and the erase hand-back takes
 * it away after the lift ([InkEraseHandBack]): the eraser end with the pen tool, and the eraser
 * tool's tip and eraser end, for the stroke and pixel types. The lasso tool erases with render
 * paused, and the lasso type still pauses at pen-down.
 *
 * @param erasing whether the SDK reports the gesture as the eraser end.
 * @param eraser whether the eraser tool is active: its tip erases as well.
 */
internal fun eraseTrace(erasing: Boolean, eraser: Boolean, interaction: Boolean, eraserType: String): Boolean =
    (erasing || eraser) && !interaction && eraserType != LASSO_TYPE

/**
 * Whether the active tool wants firmware ink rather than only its raw points. The eraser tool
 * renders as stock's does, the soft-eraser trace of its main style; with the lasso type it keeps
 * render off. The lasso tool renders only its free-form selection trace, and only while nothing
 * is selected.
 */
internal fun toolRenders(
    eraser: Boolean,
    eraserType: String,
    interaction: Boolean,
    fastLasso: Boolean,
    hasSelection: Boolean,
): Boolean = if (eraser) eraserType != LASSO_TYPE else !interaction || (fastLasso && !hasSelection)

/** The painter the tip draws with: the tool's main style, as the resume sends it. */
internal fun toolStrokeStyle(eraser: Boolean, fastLasso: Boolean, eraserType: String): Int = when {
    fastLasso -> PenStyle.DASH
    eraser && eraserType != LASSO_TYPE -> PenStyle.SOFT_ERASER
    else -> PenStyle.FOUNTAIN
}

/** The painter the eraser end draws with, which it writes into the style slot on touch-down. */
internal fun eraserEndStyle(eraserType: String): Int =
    if (eraserType == LASSO_TYPE) PenStyle.DASH else PenStyle.SOFT_ERASER

/**
 * Whether an eraser-end contact with render on leaves the style slot holding something other than
 * the tool's own style. With the eraser tool both are the soft eraser, so its tip needs no re-send.
 */
internal fun eraserEndOverwritesStyle(eraser: Boolean, fastLasso: Boolean, eraserType: String): Boolean =
    toolStrokeStyle(eraser, fastLasso, eraserType) != eraserEndStyle(eraserType)

/**
 * Whether the SDK's pen-up refresh may start a reconcile. The eraser tool's tip strokes are ordinary
 * strokes to the SDK, so its pen-up timer fires about 500 ms after each lift; where the erase
 * hand-back owns those gestures, a reconcile would put a repaint-mode frame inside the hand-back.
 * Stock's consumer for its eraser tool does nothing either. The lasso type keeps today's path.
 */
internal fun penUpRefreshReconciles(eraser: Boolean, interaction: Boolean, eraserType: String): Boolean =
    !(eraser && eraseTrace(erasing = false, eraser = true, interaction = interaction, eraserType = eraserType))

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
 * The eraser tool with the stroke or pixel type makes the soft eraser its main style and sends no
 * width, as stock does on selecting its eraser (`SET_STROKE_STYLE [8]`, no `SET_STROKE_WIDTH`): the
 * trace takes its width from the soft eraser's parameters. With the lasso type it keeps the pen's
 * settings and renders nothing.
 *
 * The eraser end is drawn by the firmware for the stroke and pixel types. It is off while the
 * lasso tool is active, which erases with render paused: SurfaceFlinger then picks the reader that
 * paints nothing and leaves the style slot alone, so the next lasso stroke stays dashed. It is also
 * off for the lasso type, whose eraser end still pauses at pen-down.
 */
internal fun penSettingsCalls(
    fastLasso: Boolean,
    interaction: Boolean,
    eraser: Boolean,
    eraserType: String,
    eraserPx: Float,
    penPx: Float,
): List<PenCall> {
    val lassoType = eraserType == LASSO_TYPE
    val style = toolStrokeStyle(eraser, fastLasso, eraserType)
    return buildList {
        add(PenCall.StrokeParameters(PenStyle.DASH, listOf(LASSO_DASH)))
        add(PenCall.StrokeParameters(PenStyle.SOFT_ERASER, listOf(eraserPx, ERASER_OPACITY, ERASER_BLACK_OPACITY)))
        when (style) {
            PenStyle.DASH -> add(PenCall.StrokeWidth(LASSO_DASH))
            PenStyle.FOUNTAIN -> add(PenCall.StrokeWidth(penPx))
        }
        add(PenCall.StrokeColor(INK_BLACK))
        add(PenCall.StrokeStyle(style))
        add(PenCall.EraserTrace(!interaction && !lassoType, eraserEndStyle(eraserType)))
        add(PenCall.BrushTrace(true))
    }
}
