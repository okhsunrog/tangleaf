package dev.okhsunrog.mobile_system

import org.junit.Assert.*
import org.junit.Test

class InkHandBackCoverageTest {
    private val sheet = InkBounds(69.0, 462.0, 1792.0, 2874.2)
    private val limit = InkBounds(69.0, 462.0, 1792.0, 2479.0)

    @Test fun sheetUnitsMapToViewPixelsTheWayTheRegionIsFed() {
        // 1723 px across 1000 units, 2412.2 px down 1400 units: 1.723 px per unit both ways.
        val view = InkHandBackCoverage.sheetToView(sheet, 1000.0, 1400.0, InkBounds(100.0, 200.0, 300.0, 400.0), limit)!!
        assertEquals(69 + 172.3, view.left, 1e-9)
        assertEquals(462 + 344.6, view.top, 1e-9)
        assertEquals(69 + 516.9, view.right, 1e-9)
        assertEquals(462 + 689.2, view.bottom, 1e-9)
    }

    @Test fun whatIsOffThePanelIsClippedAndNothingLeftIsNoBounds() {
        // Below the visible limit: clipped to it.
        val clipped = InkHandBackCoverage.sheetToView(sheet, 1000.0, 1400.0, InkBounds(0.0, 1100.0, 1000.0, 1400.0), limit)!!
        assertEquals(2479.0, clipped.bottom, 0.0)
        assertNull(InkHandBackCoverage.sheetToView(sheet, 1000.0, 1400.0, InkBounds(0.0, 1250.0, 1000.0, 1400.0), limit))
        // Empty and not finite: nothing was erased.
        assertNull(InkHandBackCoverage.sheetToView(sheet, 1000.0, 1400.0, InkBounds(0.0, 0.0, 0.0, 0.0), limit))
        assertNull(InkHandBackCoverage.sheetToView(sheet, 1000.0, 1400.0, InkBounds(Double.NaN, 0.0, 1.0, 1.0), limit))
    }

    @Test fun aRegionHoldingTheErasedBoundsIsCoveredAndOneShortOfThemIsFlagged() {
        val coverage = InkHandBackCoverage()
        coverage.add(InkBounds(300.0, 800.0, 400.0, 900.0))
        coverage.add(InkBounds(350.0, 850.0, 500.0, 950.5))
        assertEquals(InkBounds(300.0, 800.0, 500.0, 950.5), coverage.erased)
        val holds = InkReconcileRegion.Taken(InkBounds(290.0, 790.0, 510.0, 960.0), wholeRegion = false)
        assertEquals("region=[290,790][510,960] union erased=[300,800][500,951]", coverage.describe(holds))
        // Half a pixel short is rounding, not a miss.
        assertTrue(InkHandBackCoverage.covers(InkBounds(300.5, 800.0, 500.0, 950.0), InkBounds(300.0, 800.0, 500.0, 950.5)))
        val short = InkReconcileRegion.Taken(InkBounds(290.0, 790.0, 450.0, 960.0), wholeRegion = false)
        assertEquals("region=[290,790][450,960] union erased=[300,800][500,951] UNCOVERED", coverage.describe(short))
    }

    @Test fun thePresentedFrameConsumesTheBoundsAndMissingPiecesAreNamed() {
        val coverage = InkHandBackCoverage()
        val whole = InkReconcileRegion.Taken(limit, wholeRegion = true)
        assertEquals("region=[69,462][1792,2479] whole erased=none", coverage.presented(whole))
        coverage.add(InkBounds(300.0, 800.0, 400.0, 900.0))
        // No frame could be submitted: nothing to flag, the bounds are kept for the frame to come.
        assertEquals("region=none erased=[300,800][400,900]", coverage.describe(null))
        assertEquals("region=[69,462][1792,2479] whole erased=[300,800][400,900]", coverage.presented(whole))
        assertNull(coverage.erased)
        coverage.add(InkBounds(1.0, 2.0, 3.0, 4.0))
        coverage.reset()
        assertNull(coverage.erased)
    }
}
