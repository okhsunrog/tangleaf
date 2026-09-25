package dev.okhsunrog.mobile_system

import kotlin.math.ceil
import kotlin.math.floor

/**
 * Diagnostics for the erase hand-back, and nothing else: whether the frame it presented was taken
 * for a region that holds what the page erased. The page reports the bounds of the strokes each
 * erase removed with the commit that carries the erase; the hand-back's log line puts them next to
 * that region and flags a region that does not contain them. Nothing here changes what is
 * refreshed.
 *
 * The region is the plugin's own account of what the panel owes ([InkReconcileRegion]). The frame
 * itself invalidates the whole WebView, and SurfaceFlinger picks the panel's partial update from
 * the compositor's damage, which the driver log shows and this cannot.
 */
internal class InkHandBackCoverage {
    /** What the page erased since the last hand-back frame reached the panel, in view pixels. */
    var erased: InkBounds? = null
        private set

    /** The page erased strokes within [bounds], in view pixels. */
    fun add(bounds: InkBounds) {
        erased = erased?.let { union(it, bounds) } ?: bounds
    }

    /**
     * One log fragment: the region [frame] was taken for, or `none` when no frame could be
     * submitted, then the erased bounds, and `UNCOVERED` when the region does not contain them.
     */
    fun describe(frame: InkReconcileRegion.Taken?): String {
        val region = frame?.let { "${format(it.bounds)} ${if (it.wholeRegion) "whole" else "union"}" } ?: "none"
        val pending = erased
        val flag = if (frame != null && pending != null && !covers(frame.bounds, pending)) " UNCOVERED" else ""
        return "region=$region erased=${pending?.let(::format) ?: "none"}$flag"
    }

    /** The hand-back frame reached the panel: describe it, and start over for the next one. */
    fun presented(frame: InkReconcileRegion.Taken?): String = describe(frame).also { erased = null }

    fun reset() {
        erased = null
    }

    companion object {
        /** A region short of the erased bounds by less than this still counts as holding them. */
        const val SLACK_PX = 1.0

        fun covers(region: InkBounds, erased: InkBounds): Boolean =
            region.left <= erased.left + SLACK_PX && region.top <= erased.top + SLACK_PX &&
                region.right >= erased.right - SLACK_PX && region.bottom >= erased.bottom - SLACK_PX

        /**
         * Sheet units to view pixels, the mapping the reconcile region is fed with, clipped to
         * [clip] (the drawable limit: what is off the panel cannot be left on it). Null for bounds
         * that are empty, not finite, or wholly clipped away.
         */
        fun sheetToView(sheet: InkBounds, unitsWide: Double, unitsHigh: Double, units: InkBounds, clip: InkBounds): InkBounds? {
            if (!listOf(units.left, units.top, units.right, units.bottom).all { it.isFinite() }) return null
            if (units.right <= units.left || units.bottom <= units.top) return null
            val width = sheet.right - sheet.left
            val height = sheet.bottom - sheet.top
            val left = maxOf(clip.left, sheet.left + units.left.coerceIn(0.0, unitsWide) * width / unitsWide)
            val top = maxOf(clip.top, sheet.top + units.top.coerceIn(0.0, unitsHigh) * height / unitsHigh)
            val right = minOf(clip.right, sheet.left + units.right.coerceIn(0.0, unitsWide) * width / unitsWide)
            val bottom = minOf(clip.bottom, sheet.top + units.bottom.coerceIn(0.0, unitsHigh) * height / unitsHigh)
            return if (right <= left || bottom <= top) null else InkBounds(left, top, right, bottom)
        }

        /** `[left,top][right,bottom]` in whole pixels, grown outwards, as `Rect.toShortString`. */
        fun format(bounds: InkBounds): String =
            "[${floor(bounds.left).toInt()},${floor(bounds.top).toInt()}]" +
                "[${ceil(bounds.right).toInt()},${ceil(bounds.bottom).toInt()}]"

        private fun union(a: InkBounds, b: InkBounds) = InkBounds(
            minOf(a.left, b.left),
            minOf(a.top, b.top),
            maxOf(a.right, b.right),
            maxOf(a.bottom, b.bottom),
        )
    }
}
