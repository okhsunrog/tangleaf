package dev.okhsunrog.mobile_system

/**
 * Keeps pen and eraser-end gestures on the sheet away from the WebView while the firmware reads
 * the pen there.
 *
 * The raw reader gets the pen from the digitizer on its own, but Android still delivers the same
 * contact to the WebView as MotionEvents. The BOOX framework patches `WebView.onTouchEvent`: any
 * vertical travel of 5 px switches the panel into A2 through `EInkHelper.beforeScroll`, with no
 * pen-state or package check, and one second after the lift `clearTransientUpdate(true)` puts the
 * scheme back with a full-screen GC16. While the ink session is running SurfaceFlinger ignores the
 * A2 request; while it is paused (the eraser, a lasso) the panel really goes to A2 and the lift
 * ends in the full-screen flash. Recorded on the Note Air 4C, 2026-09-25
 * (`docs/onyx-reversing/stock-notes/11-observations.md`). Stock Notes never lets the pen reach a
 * scrollable view, so the framework never sees a scroll.
 *
 * The decision is taken once at the down event and held for the whole gesture, so a stroke that
 * leaves the sheet is not handed to the WebView halfway through. A pen landing on a floating
 * control ([inkOverlayHit]) still reaches the DOM, as does anything outside the drawing region and
 * every finger.
 */
internal class InkStylusRouting {
    private var consuming = false

    /**
     * @param down whether this event starts a gesture (ACTION_DOWN).
     * @param end whether it ends one (ACTION_UP or ACTION_CANCEL).
     * @param stylus whether the pointer is a stylus tip or eraser end.
     * @param active whether the ink session is currently reading the pen on the sheet.
     * @return true when the event must not reach the WebView.
     */
    fun consume(
        down: Boolean,
        end: Boolean,
        stylus: Boolean,
        active: Boolean,
        limit: InkRect,
        overlays: List<InkRect>,
        x: Float,
        y: Float,
    ): Boolean {
        if (down) consuming = stylus && active && limit.holds(x, y) && !inkOverlayHit(overlays, x, y)
        val result = consuming
        if (end) consuming = false
        return result
    }

    fun reset() {
        consuming = false
    }
}
