package dev.okhsunrog.mobile_system

import org.junit.Assert.*
import org.junit.Test

class SafeAreaTest {
    @Test fun eachEdgeTakesTheLargerOfItsBarAndTheCutoutInCssPixels() {
        val area = SafeArea.of(
            statusBars = InsetEdges(top = 62, right = 40, left = 40),
            navigationBars = InsetEdges(top = 30, right = 10, bottom = 90, left = 0),
            cutout = InsetEdges(top = 20, right = 25, bottom = 0, left = 50),
            density = 2f,
        )
        // The status bar only ever takes the top, and the navigation bar never does.
        assertEquals(SafeArea(top = 31.0, right = 12.5, bottom = 45.0, left = 25.0), area)
    }

    @Test fun hiddenBarsLeaveOnlyTheCutout() {
        val area = SafeArea.of(InsetEdges(), InsetEdges(), InsetEdges(top = 28), density = 2.8125f)
        assertEquals((28 / 2.8125f).toDouble(), area.top, 0.0)
        assertEquals(0.0, area.bottom, 0.0)
        assertEquals(SafeArea.NONE, SafeArea.of(InsetEdges(), InsetEdges(), InsetEdges(), 2f))
    }

    @Test fun onlyAChangedAreaIsSent() {
        val changes = SafeAreaChanges()
        val windowed = SafeArea(22.0, 0.0, 0.0, 0.0)
        // The first area is always news: the page has only heard from the startup query.
        assertTrue(changes.offer(windowed))
        // The keyboard and every WebView layout pass dispatch the same insets again.
        assertFalse(changes.offer(SafeArea(22.0, 0.0, 0.0, 0.0)))
        assertTrue(changes.offer(SafeArea.NONE))
        assertFalse(changes.offer(SafeArea.NONE))
        assertTrue(changes.offer(windowed))
    }
}
