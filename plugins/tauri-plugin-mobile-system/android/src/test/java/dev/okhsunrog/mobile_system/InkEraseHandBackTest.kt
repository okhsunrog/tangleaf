package dev.okhsunrog.mobile_system

import dev.okhsunrog.mobile_system.InkEraseHandBack.Action
import dev.okhsunrog.mobile_system.InkEraseHandBack.Companion.MICRO_CONTACT_MS
import dev.okhsunrog.mobile_system.InkEraseHandBack.Companion.contactMillis
import dev.okhsunrog.mobile_system.InkEraseHandBack.State
import org.junit.Assert.*
import org.junit.Test

class InkEraseHandBackTest {
    /** An ordinary erase: long enough never to be held. */
    private val erase = 300L

    /** A hand-back that has posted, paused and submitted its frame. */
    private fun paused(submitted: Boolean = true): InkEraseHandBack {
        val handBack = InkEraseHandBack()
        assertEquals(Action.AWAIT, handBack.eraseEnded(erase))
        assertEquals(Action.POST, handBack.canvasReady(covers = true))
        assertEquals(Action.PAUSE, handBack.settleElapsed(drawing = false))
        assertEquals(Action.NONE, handBack.frameSubmitted(submitted))
        assertEquals(State.PAUSED, handBack.state)
        return handBack
    }

    /** A hand-back held after a short contact. */
    private fun held(): InkEraseHandBack = InkEraseHandBack().apply {
        assertEquals(Action.HOLD, eraseEnded(6))
        assertEquals(State.HELD, state)
    }

    /** Every event a late timer or callback could still deliver. */
    private fun lateEvents(handBack: InkEraseHandBack): List<Action> = listOf(
        handBack.holdElapsed(drawing = false),
        handBack.canvasTimedOut(drawing = false),
        handBack.settleElapsed(drawing = false),
        handBack.frameSubmitted(true),
        handBack.framePresented(),
        handBack.resumeDelayElapsed(drawing = false),
        handBack.hardCapElapsed(),
    )

    @Test fun postPauseFrameAndDelayGiveExactlyOneReSendInEitherOrder() {
        val first = paused()
        assertEquals(Action.NONE, first.framePresented())
        assertEquals(Action.RESEND, first.resumeDelayElapsed(drawing = false))
        assertEquals(State.IDLE, first.state)

        val second = paused()
        assertEquals(Action.NONE, second.resumeDelayElapsed(drawing = false))
        assertEquals(Action.RESEND, second.framePresented())
        assertEquals(State.IDLE, second.state)

        for (handBack in listOf(first, second)) {
            assertTrue(lateEvents(handBack).all { it == Action.NONE })
            assertEquals(Action.PASS, handBack.canvasReady(covers = true))
        }
    }

    @Test fun aPenDownBeforeTheRenderOffCancelsAndNothingIsReSentAfterwards() {
        val waiting = InkEraseHandBack()
        waiting.eraseEnded(erase)
        assertEquals(Action.CANCEL, waiting.penDown())
        assertEquals(State.IDLE, waiting.state)
        assertTrue(lateEvents(waiting).all { it == Action.NONE })

        val posted = InkEraseHandBack()
        posted.eraseEnded(erase)
        assertEquals(Action.POST, posted.canvasReady(covers = true))
        assertEquals(Action.CANCEL, posted.penDown())
        assertTrue(lateEvents(posted).all { it == Action.NONE })
        // Its canvas is the reconcile's again.
        assertEquals(Action.PASS, posted.canvasReady(covers = true))
    }

    @Test fun aPenDownWhilePausedReSendsOnce() {
        val handBack = paused()
        assertEquals(Action.RESEND, handBack.penDown())
        assertEquals(Action.NONE, handBack.penDown())
        assertTrue(lateEvents(handBack).all { it == Action.NONE })
    }

    @Test fun aLifecycleResetInAnyStateDoesNothingAndSilencesEveryLaterTimer() {
        val states = listOf(
            held(),
            held().apply { canvasReady(covers = true) },
            InkEraseHandBack().apply { eraseEnded(erase) },
            InkEraseHandBack().apply { eraseEnded(erase); canvasReady(covers = true) },
            paused(),
        )
        for (handBack in states) {
            handBack.reset()
            assertEquals(State.IDLE, handBack.state)
            assertFalse(handBack.active)
            assertTrue(lateEvents(handBack).all { it == Action.NONE })
            assertEquals(Action.NONE, handBack.penDown())
        }
    }

    @Test fun aCanvasThatNeverComesStillHandsBack() {
        val handBack = InkEraseHandBack()
        handBack.eraseEnded(erase)
        assertEquals(Action.POST, handBack.canvasTimedOut(drawing = false))
        assertEquals(State.POSTED, handBack.state)
        assertEquals(Action.PAUSE, handBack.settleElapsed(drawing = false))
    }

    @Test fun withNoFrameToSubmitTheDelayAloneReSends() {
        val handBack = paused(submitted = false)
        assertEquals(Action.RESEND, handBack.resumeDelayElapsed(drawing = false))
    }

    @Test fun theHardCapReSendsEvenWithoutTheFrame() {
        val handBack = paused()
        assertEquals(Action.NONE, handBack.resumeDelayElapsed(drawing = false))
        assertEquals(Action.RESEND, handBack.hardCapElapsed())
        assertEquals(Action.NONE, handBack.framePresented())
    }

    @Test fun aNewerCanvasWhilePausedIsResubmittedNotPostedAgain() {
        val handBack = paused()
        assertEquals(Action.RESUBMIT, handBack.canvasReady(covers = true))
        assertEquals(State.PAUSED, handBack.state)
        // The resubmitted frame is the one the re-send waits for.
        assertEquals(Action.NONE, handBack.frameSubmitted(true))
        assertEquals(Action.NONE, handBack.resumeDelayElapsed(drawing = false))
        assertEquals(Action.RESEND, handBack.framePresented())
    }

    @Test fun aResubmissionThatFindsNothingAfterTheDelayReSendsAtOnce() {
        val handBack = paused()
        assertEquals(Action.NONE, handBack.resumeDelayElapsed(drawing = false))
        assertEquals(Action.RESUBMIT, handBack.canvasReady(covers = true))
        assertEquals(Action.RESEND, handBack.frameSubmitted(false))
    }

    @Test fun aCanvasAfterThePostIsConsumedWithoutASecondPost() {
        val handBack = InkEraseHandBack()
        handBack.eraseEnded(erase)
        assertEquals(Action.POST, handBack.canvasReady(covers = true))
        assertEquals(Action.NONE, handBack.canvasReady(covers = true))
        assertEquals(State.POSTED, handBack.state)
        assertEquals(Action.PAUSE, handBack.settleElapsed(drawing = false))
    }

    @Test fun aCanvasOlderThanTheLastStrokeIsDroppedAndPostsNothing() {
        val handBack = InkEraseHandBack()
        handBack.eraseEnded(erase)
        assertEquals(Action.NONE, handBack.canvasReady(covers = false))
        assertEquals(State.AWAIT_CANVAS, handBack.state)
        assertEquals(Action.POST, handBack.canvasReady(covers = true))
        assertEquals(Action.NONE, paused().canvasReady(covers = false))
    }

    @Test fun aTimerUnderAPenOnTheGlassNeverTurnsRenderOff() {
        val waiting = InkEraseHandBack().apply { eraseEnded(erase) }
        assertEquals(Action.CANCEL, waiting.canvasTimedOut(drawing = true))

        val posted = InkEraseHandBack().apply { eraseEnded(erase); canvasReady(covers = true) }
        assertEquals(Action.CANCEL, posted.settleElapsed(drawing = true))
        assertEquals(State.IDLE, posted.state)

        assertEquals(Action.RESEND, paused().resumeDelayElapsed(drawing = true))

        val holding = held()
        assertEquals(Action.CANCEL, holding.holdElapsed(drawing = true))
        assertEquals(State.IDLE, holding.state)
    }

    @Test fun anEraseEndingWhileOneIsAwaitingRestartsIt() {
        val handBack = InkEraseHandBack()
        handBack.eraseEnded(erase)
        handBack.canvasReady(covers = true)
        assertEquals(Action.AWAIT, handBack.eraseEnded(erase))
        assertEquals(State.AWAIT_CANVAS, handBack.state)
        // While paused the pending re-send covers the new erase; its timers must stay.
        assertEquals(Action.NONE, paused().eraseEnded(erase))
        assertEquals(Action.NONE, paused().eraseEnded(6))
    }

    @Test fun theSettleIsMeasuredFromTheStartOfThePostCall() {
        assertEquals(15L, InkEraseHandBack.settleDelay(155))
        assertEquals(168L, InkEraseHandBack.settleDelay(2))
        assertEquals(0L, InkEraseHandBack.settleDelay(400))
    }

    @Test fun onlyAContactShorterThanTheMicroContactLimitIsHeld() {
        assertEquals(Action.HOLD, InkEraseHandBack().eraseEnded(0))
        assertEquals(Action.HOLD, InkEraseHandBack().eraseEnded(MICRO_CONTACT_MS - 1))
        assertEquals(Action.AWAIT, InkEraseHandBack().eraseEnded(MICRO_CONTACT_MS))
        // A synthesized end has no length and is handed back as it stands.
        assertEquals(Action.AWAIT, InkEraseHandBack().eraseEnded(null))
    }

    /** The eraser end as the tablet delivers it: a 6 ms contact, the real one 84 ms later. */
    @Test fun theRealContactAfterAShortOneCancelsTheHoldAndGetsTheOnlyHandBack() {
        val handBack = InkEraseHandBack()
        val downStamp = 1_758_800_000_000L
        val shortContact = contactMillis(downStamp, downStamp + 6, beginUptime = 500_000, endUptime = 500_007)
        assertEquals(6L, shortContact)
        assertEquals(Action.HOLD, handBack.eraseEnded(shortContact))
        // The short contact's erased canvas lands during the hold: kept, never the reconcile's.
        assertEquals(Action.NONE, handBack.canvasReady(covers = true))
        // The real contact touches down at +90 ms, before the hold timer (armed at the lift for
        // MICRO_CONTACT_HOLD_MS) runs, and cancels it. No post has happened, so SurfaceFlinger is
        // still in the handwriting schema for it.
        assertEquals(Action.CANCEL, handBack.penDown())
        assertEquals(State.IDLE, handBack.state)
        assertTrue(lateEvents(handBack).all { it == Action.NONE })
        // The real contact lasts 310 ms and is handed back with stock's timing.
        val realContact = contactMillis(downStamp + 90, downStamp + 400, beginUptime = 500_090, endUptime = 500_400)
        assertEquals(Action.AWAIT, handBack.eraseEnded(realContact))
        assertEquals(Action.POST, handBack.canvasReady(covers = true))
        assertEquals(Action.PAUSE, handBack.settleElapsed(drawing = false))
    }

    @Test fun whenNothingFollowsTheHoldAKeptCanvasPostsAtOnce() {
        val handBack = held()
        assertEquals(Action.NONE, handBack.canvasReady(covers = true))
        assertEquals(Action.POST, handBack.holdElapsed(drawing = false))
        assertEquals(State.POSTED, handBack.state)
        assertEquals(Action.PAUSE, handBack.settleElapsed(drawing = false))
    }

    @Test fun whenNothingFollowsTheHoldWithoutACanvasItWaitsForOneAsUsual() {
        val handBack = held()
        // A canvas older than the last stroke is not kept.
        assertEquals(Action.NONE, handBack.canvasReady(covers = false))
        assertEquals(Action.AWAIT, handBack.holdElapsed(drawing = false))
        assertEquals(State.AWAIT_CANVAS, handBack.state)
        assertEquals(Action.POST, handBack.canvasReady(covers = true))

        val timedOut = held()
        assertEquals(Action.AWAIT, timedOut.holdElapsed(drawing = false))
        assertEquals(Action.POST, timedOut.canvasTimedOut(drawing = false))
    }

    @Test fun theHoldTimerIsTheOnlyEventThatEndsAHold() {
        val handBack = held()
        assertEquals(Action.NONE, handBack.canvasTimedOut(drawing = false))
        assertEquals(Action.NONE, handBack.settleElapsed(drawing = false))
        assertEquals(Action.NONE, handBack.resumeDelayElapsed(drawing = false))
        assertEquals(Action.NONE, handBack.hardCapElapsed())
        assertEquals(State.HELD, handBack.state)
        assertEquals(Action.AWAIT, handBack.holdElapsed(drawing = false))
        // A second run of the same timer finds nothing held.
        assertEquals(Action.NONE, handBack.holdElapsed(drawing = false))
    }

    @Test fun theContactIsTimedByTheDigitizerAndByUptimeOnlyWhenItCannotBe() {
        val stamp = 1_758_800_000_000L
        // The main thread ran the lift 300 ms late; the digitizer still says 6 ms.
        assertEquals(6L, contactMillis(stamp, stamp + 6, beginUptime = 1_000, endUptime = 1_306))
        // No timestamp on the touch-down, or stamps that run backwards: the callbacks' uptime.
        assertEquals(12L, contactMillis(0, stamp, beginUptime = 1_000, endUptime = 1_012))
        assertEquals(30L, contactMillis(stamp, stamp - 10, beginUptime = 1_000, endUptime = 1_030))
        assertEquals(0L, contactMillis(0, 0, beginUptime = 1_000, endUptime = 990))
    }
}
