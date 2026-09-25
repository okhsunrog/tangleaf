package dev.okhsunrog.mobile_system

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class InkStylusRoutingTest {
    private val limit = InkRect(0, 100, 1000, 1500)
    private val menu = InkRect(400, 400, 600, 480)

    private fun InkStylusRouting.event(
        down: Boolean = false,
        end: Boolean = false,
        stylus: Boolean = true,
        active: Boolean = true,
        x: Float = 500f,
        y: Float = 800f,
    ) = consume(down, end, stylus, active, limit, listOf(menu), x, y)

    @Test
    fun `a pen gesture on the sheet never reaches the WebView`() {
        val routing = InkStylusRouting()
        assertTrue(routing.event(down = true))
        assertTrue(routing.event())
        assertTrue(routing.event(end = true))
    }

    @Test
    fun `the gesture stays consumed when it leaves the sheet`() {
        val routing = InkStylusRouting()
        assertTrue(routing.event(down = true))
        assertTrue(routing.event(y = 50f))
        assertTrue(routing.event(end = true, y = 50f))
    }

    @Test
    fun `fingers always reach the WebView`() {
        val routing = InkStylusRouting()
        assertFalse(routing.event(down = true, stylus = false))
        assertFalse(routing.event(stylus = false))
    }

    @Test
    fun `a pen starting outside the drawing region reaches the WebView`() {
        val routing = InkStylusRouting()
        assertFalse(routing.event(down = true, y = 50f))
        assertFalse(routing.event(y = 800f))
    }

    @Test
    fun `a pen tapping a floating control reaches the WebView`() {
        val routing = InkStylusRouting()
        assertFalse(routing.event(down = true, x = 500f, y = 440f))
        assertFalse(routing.event(end = true, x = 500f, y = 440f))
    }

    @Test
    fun `nothing is consumed while the ink session is paused`() {
        val routing = InkStylusRouting()
        assertFalse(routing.event(down = true, active = false))
    }

    @Test
    fun `the next gesture decides afresh after a lift`() {
        val routing = InkStylusRouting()
        assertTrue(routing.event(down = true))
        routing.event(end = true)
        assertFalse(routing.event(down = true, active = false))
    }

    @Test
    fun `reset drops a gesture in flight`() {
        val routing = InkStylusRouting()
        assertTrue(routing.event(down = true))
        routing.reset()
        assertFalse(routing.event())
    }
}
