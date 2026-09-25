package dev.okhsunrog.mobile_system

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AutoA2SwitchTest {
    private class WebViewFlag(var value: Boolean? = true) {
        val writes = mutableListOf<Boolean>()
        val switch = AutoA2Switch(read = { value }, write = { value = it; writes += it })
    }

    @Test
    fun `an open session turns the flag off and closing it puts it back`() {
        val flag = WebViewFlag()
        flag.switch.acquire()
        assertEquals(false, flag.value)
        assertTrue(flag.switch.held)
        flag.switch.release()
        assertEquals(true, flag.value)
        assertFalse(flag.switch.held)
        assertEquals(listOf(false, true), flag.writes)
    }

    @Test
    fun `two sessions keep it off until the last one closes`() {
        val flag = WebViewFlag()
        flag.switch.acquire()
        flag.switch.acquire()
        flag.switch.release()
        assertEquals(false, flag.value)
        flag.switch.release()
        assertEquals(true, flag.value)
        assertEquals(listOf(false, true), flag.writes)
    }

    @Test
    fun `a release without an acquire changes nothing`() {
        val flag = WebViewFlag()
        flag.switch.release()
        assertEquals(true, flag.value)
        assertTrue(flag.writes.isEmpty())
    }

    @Test
    fun `a flag that was already off stays off after the session`() {
        val flag = WebViewFlag(false)
        flag.switch.acquire()
        flag.switch.release()
        assertEquals(false, flag.value)
        assertTrue(flag.writes.isEmpty())
    }

    @Test
    fun `firmware without the switch is left alone`() {
        val flag = WebViewFlag(null)
        flag.switch.acquire()
        flag.switch.release()
        assertTrue(flag.writes.isEmpty())
    }

    @Test
    fun `a new session after the last one closed saves the value again`() {
        val flag = WebViewFlag()
        flag.switch.acquire()
        flag.switch.release()
        flag.switch.acquire()
        assertEquals(false, flag.value)
        flag.switch.release()
        assertEquals(true, flag.value)
    }
}
