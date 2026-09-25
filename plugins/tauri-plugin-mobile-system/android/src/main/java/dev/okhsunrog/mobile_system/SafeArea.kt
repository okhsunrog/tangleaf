package dev.okhsunrog.mobile_system

/** Window insets of one kind, in device pixels, edge by edge. */
data class InsetEdges(val top: Int = 0, val right: Int = 0, val bottom: Int = 0, val left: Int = 0)

/**
 * The part of the window the page keeps its controls out of, in CSS px.
 *
 * The activity is edge-to-edge, so the WebView always spans the whole window and the page pads
 * itself. The status bar and a display cutout take the top; the navigation bar and a cutout take
 * the other edges. A bar that is hidden reports no insets, which is how hiding the bars reaches
 * the page.
 */
data class SafeArea(val top: Double, val right: Double, val bottom: Double, val left: Double) {
    companion object {
        val NONE = SafeArea(0.0, 0.0, 0.0, 0.0)

        fun of(statusBars: InsetEdges, navigationBars: InsetEdges, cutout: InsetEdges, density: Float) =
            SafeArea(
                top = (maxOf(statusBars.top, cutout.top) / density).toDouble(),
                right = (maxOf(navigationBars.right, cutout.right) / density).toDouble(),
                bottom = (maxOf(navigationBars.bottom, cutout.bottom) / density).toDouble(),
                left = (maxOf(navigationBars.left, cutout.left) / density).toDouble(),
            )
    }
}

/**
 * The last safe area the page was told about. Window insets are dispatched for many reasons (the
 * keyboard, every layout pass of the WebView), and the page only needs to hear when the safe area
 * itself moved.
 */
class SafeAreaChanges {
    private var last: SafeArea? = null

    /** True when [area] differs from the last one offered, which is then the one to send. */
    fun offer(area: SafeArea): Boolean {
        if (area == last) return false
        last = area
        return true
    }
}
