package dev.okhsunrog.mobile_system

import dev.okhsunrog.mobile_system.InkReconcileGate.Next
import org.junit.Assert.*
import org.junit.Test

class InkReconcileGateTest {
    @Test fun theFirstStrokeDoesNotWaitForACallbackThisFirmwareMayNeverSend() {
        val gate = InkReconcileGate()
        assertFalse(gate.penUpRefreshSeen)
        gate.began(erasing = false)
        assertEquals(Next.SOON, gate.canvasReady())
    }

    @Test fun aLatePenUpRefreshAfterTheReconcileOnlyTeachesTheGateToWaitNextTime() {
        val gate = InkReconcileGate()
        gate.began(erasing = false)
        assertEquals(Next.SOON, gate.canvasReady())
        gate.reconciled()
        assertEquals(Next.WAIT, gate.penUpRefresh()) // No frame is pending; nothing to run.
        assertTrue(gate.penUpRefreshSeen)
        gate.began(erasing = false)
        assertEquals(Next.WAIT, gate.canvasReady())
    }

    @Test fun onceSeenTheReconcileWaitsForTheStrokeUnionWhicheverSideIsLast() {
        val gate = InkReconcileGate()
        gate.penUpRefresh()
        gate.reconciled()

        gate.began(erasing = false)
        assertEquals(Next.WAIT, gate.canvasReady())
        assertEquals(Next.NOW, gate.penUpRefresh())
        gate.reconciled()

        gate.began(erasing = false)
        assertEquals(Next.WAIT, gate.penUpRefresh()) // The canvas has not acknowledged yet.
        assertEquals(Next.NOW, gate.canvasReady())
    }

    @Test fun anErasingStrokeNeverWaitsBecauseTheSdkArmsNoTimerForIt() {
        val gate = InkReconcileGate()
        gate.penUpRefresh()
        gate.reconciled()
        gate.began(erasing = true)
        assertFalse(gate.expectingPenUpRefresh)
        assertEquals(Next.SOON, gate.canvasReady())
    }

    @Test fun aPenUpRefreshThatNeverArrivesReleasesTheReconcileOnTimeout() {
        val gate = InkReconcileGate()
        gate.penUpRefresh()
        gate.reconciled()
        gate.began(erasing = false)
        assertEquals(Next.WAIT, gate.canvasReady())
        assertTrue(gate.expectingPenUpRefresh)
        assertEquals(Next.SOON, gate.timedOut())
        assertFalse(gate.expectingPenUpRefresh)
    }

    @Test fun aTimeoutWithNothingAcknowledgedStillWaits() {
        val gate = InkReconcileGate()
        gate.penUpRefresh()
        gate.reconciled()
        gate.began(erasing = false)
        assertEquals(Next.WAIT, gate.timedOut())
    }

    @Test fun aNewGestureDropsWhateverThePreviousOneLeftHalfAcknowledged() {
        val gate = InkReconcileGate()
        gate.penUpRefresh()
        gate.reconciled()
        gate.began(erasing = false)
        gate.canvasReady()
        gate.began(erasing = false)
        assertEquals(Next.WAIT, gate.penUpRefresh())
    }

    @Test fun aFrameCommittedWithNoGestureAtAllReconcilesWithoutWaiting() {
        val gate = InkReconcileGate()
        gate.penUpRefresh()
        gate.reconciled()
        // Undo, a decoration change, a tool switch: the page commits with no stroke behind it.
        assertEquals(Next.SOON, gate.canvasReady())
    }
}
