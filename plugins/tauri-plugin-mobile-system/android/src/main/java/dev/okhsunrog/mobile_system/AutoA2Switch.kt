package dev.okhsunrog.mobile_system

/**
 * Holds the BOOX WebView's `autoA2ForScrolling` off while an ink session is open.
 *
 * The firmware patches `WebView.onTouchEvent`: any vertical travel of 5 px counts as a scroll and
 * switches the panel to A2, and one second after the lift the A2 is cleared with a full-screen GC16.
 * Pen and finger taps on the handwriting toolbar move that much often enough to flash the panel on
 * about every other tool switch (13 flashes in 41 switches recorded on 2026-09-26; none with the
 * flag off). The flag is a plain field of this WebView object, so turning it off affects this
 * process only and dies with it; the WebView constructor sets it back to true.
 *
 * Sessions acquire and release it; the first acquire saves the current value and turns it off, the
 * last release puts the saved value back. When the firmware has no such method, [read] returns null
 * and nothing is ever written.
 */
internal class AutoA2Switch(
    private val read: () -> Boolean?,
    private val write: (Boolean) -> Unit,
    private val log: (String) -> Unit = {},
) {
    private var holders = 0
    private var saved: Boolean? = null

    val held: Boolean get() = holders > 0

    fun acquire() {
        if (holders++ > 0) return
        saved = read()
        val before = saved ?: return log("autoA2ForScrolling unavailable")
        if (before) write(false)
        log("autoA2ForScrolling $before -> false")
    }

    fun release() {
        if (holders == 0) return
        if (--holders > 0) return
        val restore = saved ?: return
        saved = null
        if (restore) write(true)
        log("autoA2ForScrolling false -> $restore")
    }
}
