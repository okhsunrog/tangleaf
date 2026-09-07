package dev.okhsunrog.mobile_system

/**
 * When the reconcile may run: stock's two-sided rendezvous.
 *
 * `EpdShapeHandler.P0()` is called from two places and only does anything once both have happened:
 * the SDK's pen-up timer (`RawInputReader.I()` → `onPenUpRefresh`, `penUpRefreshTimeMs`, 500 ms by
 * default) and the completion of the flush that puts the stroke into the app's own bitmap. Ours is
 * the same shape: the pen-up refresh, and the web canvas acknowledging the frame that holds the
 * stroke. Whichever is last starts the reconcile.
 *
 * Two things the stock app never has to handle:
 *
 * * An erasing gesture gets no pen-up refresh at all — `RawInputReader.M()` returns before arming
 *   the timer when the gesture was an erase — so we must not wait for one.
 * * A firmware that never delivers the callback would deadlock the reconcile, so we only wait for
 *   a pen-up refresh after this session has seen one, and the caller arms a timeout as well.
 */
internal class InkReconcileGate {
    enum class Next {
        /** Nothing to do yet. */
        WAIT,

        /** No pen-up refresh is coming for this gesture: reconcile on the ordinary frame delay. */
        SOON,

        /** Both sides are in: reconcile now. */
        NOW,
    }

    /** True once this firmware has delivered a pen-up refresh; before that we never wait for one. */
    var penUpRefreshSeen = false
        private set

    private var expecting = false
    private var canvasReady = false
    private var penUpReady = false

    /** True while a pen-up refresh is still owed for the gesture that just ended. */
    val expectingPenUpRefresh: Boolean
        get() = expecting

    /** A gesture started. Its ink supersedes whatever the previous one left half-reconciled. */
    fun began(erasing: Boolean) {
        canvasReady = false
        penUpReady = false
        expecting = !erasing && penUpRefreshSeen
    }

    /** The web canvas acknowledged a frame carrying the stroke. */
    fun canvasReady(): Next {
        canvasReady = true
        return next()
    }

    /** The SDK's pen-up timer fired and handed us the stroke union. */
    fun penUpRefresh(): Next {
        penUpRefreshSeen = true
        penUpReady = true
        expecting = false
        return next()
    }

    /** The pen-up refresh did not arrive in the window we allowed it. */
    fun timedOut(): Next {
        expecting = false
        return next()
    }

    private fun next(): Next = when {
        !canvasReady -> Next.WAIT
        penUpReady -> Next.NOW
        expecting -> Next.WAIT
        else -> Next.SOON
    }

    /** The reconcile ran; the next one needs a fresh frame. */
    fun reconciled() {
        canvasReady = false
        penUpReady = false
        expecting = false
    }

    fun reset() = reconciled()
}
