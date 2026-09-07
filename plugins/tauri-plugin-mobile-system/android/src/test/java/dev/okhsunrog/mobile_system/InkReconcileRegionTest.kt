package dev.okhsunrog.mobile_system

import org.junit.Assert.*
import org.junit.Test

class InkReconcileRegionTest {
    private val limit = InkBounds(0.0, 0.0, 1000.0, 1400.0)

    /** A fresh region owes the whole panel once; spend that so the union rules are visible. */
    private fun armed(): InkReconcileRegion = InkReconcileRegion().apply { take(limit) }

    @Test fun aFreshRegionOwesTheWholeWritingAreaBeforeAnyStroke() {
        val region = InkReconcileRegion()
        assertTrue(region.wholeRegionPending)
        val taken = region.take(limit)
        assertEquals(InkReconcileRegion.Taken(limit, true), taken)
        assertFalse(region.wholeRegionPending)
        assertNull(region.take(limit))
    }

    @Test fun consecutiveReconcilesOverlapBecauseThePreviousStrokeIsIncluded() {
        val region = armed()
        region.add(100.0, 100.0, 200.0, 200.0)
        assertEquals(InkBounds(100.0, 100.0, 200.0, 200.0), region.take(limit)?.bounds)
        region.add(600.0, 600.0, 700.0, 700.0)
        // The second repaint still covers where the first one drew.
        assertEquals(InkBounds(100.0, 100.0, 700.0, 700.0), region.take(limit)?.bounds)
        region.add(650.0, 650.0, 660.0, 660.0)
        assertEquals(InkBounds(600.0, 600.0, 700.0, 700.0), region.take(limit)?.bounds)
    }

    @Test fun aStrokeIsTheUnionOfEveryPointOfferedForIt() {
        val region = armed()
        region.add(10.0, 20.0, 30.0, 40.0)
        region.add(500.0, 5.0, 510.0, 15.0)
        region.add(Double.NaN, 0.0, 1.0, 1.0)
        region.add(50.0, 50.0, 50.0, 60.0)
        assertEquals(InkBounds(10.0, 5.0, 510.0, 40.0), region.take(limit)?.bounds)
    }

    @Test fun anythingDrawingOverThePanelForcesOneWholeRegionRepaintThenReArms() {
        val region = armed()
        region.add(100.0, 100.0, 120.0, 120.0)
        region.invalidateAll()
        val fallback = region.take(limit)
        assertEquals(InkReconcileRegion.Taken(limit, true), fallback)
        // The stroke that was pending is covered by the whole-region repaint, and the next one
        // goes back to the union that includes it.
        region.add(800.0, 800.0, 820.0, 820.0)
        assertEquals(InkBounds(100.0, 100.0, 820.0, 820.0), region.take(limit)?.bounds)
    }

    @Test fun theWholeRegionIsOwedEvenWhenNoStrokeIsPending() {
        val region = armed()
        assertNull(region.take(limit))
        region.invalidateAll()
        assertEquals(InkReconcileRegion.Taken(limit, true), region.take(limit))
    }

    @Test fun regionsAreClippedToTheDrawableLimitAndDroppedWhenNothingIsVisible() {
        val region = armed()
        region.add(-50.0, -50.0, 40.0, 40.0)
        assertEquals(InkBounds(0.0, 0.0, 40.0, 40.0), region.take(limit)?.bounds)
        region.add(2000.0, 2000.0, 2100.0, 2100.0)
        // Off-panel on its own, but the previous stroke is still inside.
        assertEquals(InkBounds(0.0, 0.0, 1000.0, 1400.0), region.take(limit)?.bounds)
        region.add(2000.0, 2000.0, 2100.0, 2100.0)
        assertNull(region.take(limit))
    }

    @Test fun anEmptyLimitReconcilesNothingAndKeepsWhatWasPending() {
        val region = InkReconcileRegion()
        region.add(10.0, 10.0, 20.0, 20.0)
        assertNull(region.take(InkBounds(0.0, 0.0, 0.0, 0.0)))
        assertTrue(region.wholeRegionPending)
        assertEquals(InkReconcileRegion.Taken(limit, true), region.take(limit))
    }

    @Test fun aFrameThatNeverReachedThePanelPutsItsRegionBack() {
        val region = armed()
        region.add(100.0, 100.0, 200.0, 200.0)
        val taken = region.take(limit)!!
        region.putBack(taken)
        assertEquals(InkBounds(100.0, 100.0, 200.0, 200.0), region.take(limit)?.bounds)
    }

    @Test fun aWholeRegionFrameThatNeverReachedThePanelStaysOwed() {
        val region = InkReconcileRegion()
        val taken = region.take(limit)!!
        assertTrue(taken.wholeRegion)
        assertFalse(region.wholeRegionPending)
        region.putBack(taken)
        assertTrue(region.wholeRegionPending)
        assertEquals(InkReconcileRegion.Taken(limit, true), region.take(limit))
    }

    @Test fun aResetSessionOwesTheWholePanelAgainAndForgetsTheOldStrokes() {
        val region = armed()
        region.add(100.0, 100.0, 200.0, 200.0)
        region.reset()
        assertTrue(region.wholeRegionPending)
        assertEquals(InkReconcileRegion.Taken(limit, true), region.take(limit))
        region.add(800.0, 800.0, 820.0, 820.0)
        assertEquals(InkBounds(800.0, 800.0, 820.0, 820.0), region.take(limit)?.bounds)
    }
}
