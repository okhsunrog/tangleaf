package dev.okhsunrog.mobile_system

/**
 * What follows an erase whose trace the firmware drew: stock Notes' hand-back, as a pure state
 * machine. The caller owns the clock, the timers and every firmware call; this only decides.
 *
 * Stock keeps the session running while the eraser is on the glass and SurfaceFlinger paints the
 * soft-eraser trace. After the lift it posts the erased result with `ENABLE_POST(1)` while the
 * session still runs, turns render off (input stays on) about 170 ms after that call started,
 * invalidates its view once, and about 300 ms later re-sends the whole pen configuration and
 * turns render on again (`EraseFinishAction`, `InvalidateViewWithPenControlAction`,
 * `ResumeRawDrawingRequest`; recorded in `docs/onyx-reversing/stock-notes/11-observations.md`).
 * The re-send is what puts the pen's own style back into SurfaceFlinger's single style slot, which
 * the eraser end overwrote.
 *
 * `IDLE → AWAIT_CANVAS → POSTED → PAUSED → IDLE`. A pen-down leaves it at once: before the post
 * with render still on (`CANCEL`), after it with the re-send (`RESEND`). Render never goes off
 * under a pen on the glass, and it never stays off: the paused state always ends in `RESEND`,
 * through the frame and the delay, a pen-down, or the hard cap.
 *
 * A contact shorter than [MICRO_CONTACT_MS] goes through `HELD` first. On this tablet the eraser
 * end lands as a contact of a few milliseconds followed by the real one 30-155 ms later; stock's
 * recordings never show it and the cause is unknown. Started at once, the hand-back could reach
 * `ENABLE_POST(1)`, which blocks the main thread about 150 ms and takes SurfaceFlinger out of the
 * handwriting schema, while the real contact was already on the glass. So the short contact's
 * hand-back waits [MICRO_CONTACT_HOLD_MS], and a pen-down in that time cancels it like any other.
 */
internal class InkEraseHandBack {
    enum class State { IDLE, HELD, AWAIT_CANVAS, POSTED, PAUSED }

    enum class Action {
        /** Nothing to do, and the event is used up. */
        NONE,

        /** Not the hand-back's business: the caller goes on as if there were none. */
        PASS,

        /** A short contact ended: arm the hold, and nothing else yet. */
        HOLD,

        /** Wait for the erased canvas: arm the canvas timeout. */
        AWAIT,

        /** `ENABLE_POST(1)`, then the settle timer measured from the start of that call. */
        POST,

        /** Render off with input on, the plain frame, then the resume delay and the hard cap. */
        PAUSE,

        /** A newer canvas while paused: submit the plain frame again. */
        RESUBMIT,

        /** Re-send the tool's full pen settings and turn render on. The hand-back is over. */
        RESEND,

        /** The hand-back is dropped with render still on: remove its timers. */
        CANCEL,
    }

    var state = State.IDLE
        private set

    val active: Boolean
        get() = state != State.IDLE

    private var framePending = false
    private var delayElapsed = false

    /** A canvas holding the last native stroke was acknowledged while the hand-back was held. */
    private var heldCanvas = false

    /**
     * The eraser left the glass.
     *
     * @param contactMs how long the contact lasted, or null when that is not known.
     * @return `NONE` while paused, where the pending re-send already covers the new erase and its
     * timers must stay; `HOLD` for a contact shorter than [MICRO_CONTACT_MS]; otherwise `AWAIT`.
     * The caller replaces whatever timers the hand-back had with the one the action names.
     */
    fun eraseEnded(contactMs: Long?): Action {
        if (state == State.PAUSED) return Action.NONE
        framePending = false
        delayElapsed = false
        heldCanvas = false
        if (contactMs != null && contactMs < MICRO_CONTACT_MS) {
            state = State.HELD
            return Action.HOLD
        }
        state = State.AWAIT_CANVAS
        return Action.AWAIT
    }

    /** No contact followed the short one: the hand-back goes on as after an ordinary erase. */
    fun holdElapsed(drawing: Boolean): Action = when {
        state != State.HELD -> Action.NONE
        drawing -> penDown()
        heldCanvas -> {
            heldCanvas = false
            state = State.POSTED
            Action.POST
        }
        else -> {
            state = State.AWAIT_CANVAS
            Action.AWAIT
        }
    }

    /**
     * The web canvas acknowledged a frame.
     *
     * @param covers whether that frame holds the last native stroke, the frame fence's own test. An
     * older acknowledgement is used up without effect: posting for it would hand SurfaceFlinger a
     * canvas the settle frame then may not present.
     */
    fun canvasReady(covers: Boolean): Action = when {
        state == State.IDLE -> Action.PASS
        !covers -> Action.NONE
        // Kept for the end of the hold, which then posts at once instead of waiting for it again.
        state == State.HELD -> {
            heldCanvas = true
            Action.NONE
        }
        state == State.AWAIT_CANVAS -> {
            state = State.POSTED
            Action.POST
        }
        // The settle frame reads the fence when it runs, so it draws the newest ready canvas.
        state == State.POSTED -> Action.NONE
        else -> Action.RESUBMIT
    }

    /** The erased canvas never came; hand back without it. */
    fun canvasTimedOut(drawing: Boolean): Action = when {
        state != State.AWAIT_CANVAS -> Action.NONE
        drawing -> penDown()
        else -> {
            state = State.POSTED
            Action.POST
        }
    }

    fun settleElapsed(drawing: Boolean): Action = when {
        state != State.POSTED -> Action.NONE
        drawing -> penDown()
        else -> {
            state = State.PAUSED
            framePending = false
            delayElapsed = false
            Action.PAUSE
        }
    }

    /** The caller submitted the plain frame, or found no frame it could submit. */
    fun frameSubmitted(submitted: Boolean): Action {
        if (state != State.PAUSED) return Action.NONE
        framePending = submitted
        return ready()
    }

    fun framePresented(): Action {
        if (state != State.PAUSED) return Action.NONE
        framePending = false
        return ready()
    }

    fun resumeDelayElapsed(drawing: Boolean): Action {
        if (state != State.PAUSED) return Action.NONE
        if (drawing) return penDown()
        delayElapsed = true
        return ready()
    }

    /** Render must never stay off, whatever happened to the frame. */
    fun hardCapElapsed(): Action = if (state == State.PAUSED) finish() else Action.NONE

    /** A pen or eraser contact began. */
    fun penDown(): Action = when (state) {
        State.IDLE -> Action.NONE
        State.HELD, State.AWAIT_CANVAS, State.POSTED -> {
            reset()
            Action.CANCEL
        }
        State.PAUSED -> finish()
    }

    /** Lifecycle: no action. The next resume pushes everything and owns render. */
    fun reset() {
        state = State.IDLE
        framePending = false
        delayElapsed = false
        heldCanvas = false
    }

    private fun ready(): Action = if (!framePending && delayElapsed) finish() else Action.NONE

    private fun finish(): Action {
        reset()
        return Action.RESEND
    }

    companion object {
        /** How long the erased canvas may take to be acknowledged before the hand-back goes on. */
        const val CANVAS_TIMEOUT_MS = 750L

        /**
         * From the start of the `ENABLE_POST(1)` call to render off: 170, 168 and 170 ms in stock's
         * recordings. The call itself blocks about 150 ms when it takes SurfaceFlinger out of the
         * handwriting schema, so the remaining wait is measured from its start.
         */
        const val SETTLE_MS = 170L

        /** Stock's `RawPenArgs.resumeDelay` after an erase (`EraseFinishAction.E()`). */
        const val RESUME_DELAY_MS = 300L

        /** From render off to the latest possible re-send. */
        const val HARD_CAP_MS = 1000L

        /**
         * A contact shorter than this is the short first contact of the eraser end: 3-11 ms on the
         * tablet, while a deliberate erase, even a tap, lasts far longer.
         */
        const val MICRO_CONTACT_MS = 20L

        /**
         * How long a short contact's hand-back waits for the contact that follows it: that came
         * 30-155 ms later on the tablet.
         */
        const val MICRO_CONTACT_HOLD_MS = 200L

        /** The settle still owed once a post call that took [callMs] has returned. */
        fun settleDelay(callMs: Long): Long = (SETTLE_MS - callMs).coerceAtLeast(0L)

        /**
         * How long a contact lasted, in milliseconds. The digitizer's own timestamps (Unix epoch
         * milliseconds on the BOOX reader) come first: they are taken when the pen touched and
         * lifted, before the callbacks queue behind whatever holds the main thread. When one is
         * missing or they run backwards, the uptime at which the callbacks ran is used instead.
         */
        fun contactMillis(beginStamp: Long, endStamp: Long, beginUptime: Long, endUptime: Long): Long =
            if (beginStamp > 0 && endStamp >= beginStamp) endStamp - beginStamp
            else (endUptime - beginUptime).coerceAtLeast(0L)
    }
}
