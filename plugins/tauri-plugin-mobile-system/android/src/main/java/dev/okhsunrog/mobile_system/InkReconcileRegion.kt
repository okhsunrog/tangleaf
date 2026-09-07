package dev.okhsunrog.mobile_system

/**
 * The area the next reconcile covers, in the WebView's own pixel space.
 *
 * Stock Notes builds this rectangle in two different ways and we follow both.
 *
 * Normally it is the stroke that just ended **unioned with the one before it**
 * (`RawInputReader.i()`, which is what `onPenUpRefresh` delivers). Consecutive repaints therefore
 * always overlap, so a firmware region latched by one stroke is covered again by the next.
 *
 * After anything else has drawn over the panel — a dialog, a menu, a tool or page change — stock
 * clears `NoteDocViewInfo.isEnabledPenDirtyRect` and the next reconcile repaints the whole writing
 * region once before re-arming (`EpdShapeHandler.P0`, `DeviceReceiverEventHandler:89`). That is the
 * only escape hatch it has for a region the firmware kept but the app never covers again.
 *
 * Pure by construction so both rules are unit tested without a device.
 */
internal class InkReconcileRegion {
    /** A region handed out for one reconcile, and whether it was the whole-region fallback. */
    data class Taken(val bounds: InkBounds, val wholeRegion: Boolean)

    private var current: InkBounds? = null
    private var previous: InkBounds? = null
    private var whole = true

    /** True while the next reconcile is owed the whole writing region. */
    val wholeRegionPending: Boolean
        get() = whole

    /** Grows the stroke in progress. Non-finite and degenerate rectangles are ignored. */
    fun add(left: Double, top: Double, right: Double, bottom: Double) {
        if (!listOf(left, top, right, bottom).all { it.isFinite() }) return
        if (right <= left || bottom <= top) return
        val next = InkBounds(left, top, right, bottom)
        current = current?.let { union(it, next) } ?: next
    }

    /** Something else drew over the panel: cover everything once, exactly as stock does. */
    fun invalidateAll() {
        whole = true
    }

    /**
     * Consumes the region for one reconcile, clipped to [region] — the drawable limit. Null when
     * there is nothing to cover, which is stock's "no bean with a non-empty rect" case.
     */
    fun take(region: InkBounds): Taken? {
        if (region.right <= region.left || region.bottom <= region.top) return null
        val stroke = current
        current = null
        val taken = when {
            whole -> {
                whole = false
                Taken(region, true)
            }
            stroke == null -> null
            // The previous stroke is always included, so two repaints in a row overlap.
            else -> intersect(previous?.let { union(it, stroke) } ?: stroke, region)
                ?.let { Taken(it, false) }
        }
        if (stroke != null) previous = stroke
        return taken
    }

    /** The frame this region was taken for never reached the panel; it goes back on the pile. */
    fun putBack(taken: Taken) {
        if (taken.wholeRegion) whole = true
        else add(taken.bounds.left, taken.bounds.top, taken.bounds.right, taken.bounds.bottom)
    }

    /** A new session, or one that lost the panel: start owing the whole region again. */
    fun reset() {
        current = null
        previous = null
        whole = true
    }
}

private fun union(a: InkBounds, b: InkBounds) = InkBounds(
    minOf(a.left, b.left),
    minOf(a.top, b.top),
    maxOf(a.right, b.right),
    maxOf(a.bottom, b.bottom),
)

private fun intersect(a: InkBounds, b: InkBounds): InkBounds? {
    val left = maxOf(a.left, b.left)
    val top = maxOf(a.top, b.top)
    val right = minOf(a.right, b.right)
    val bottom = minOf(a.bottom, b.bottom)
    return if (right <= left || bottom <= top) null else InkBounds(left, top, right, bottom)
}
