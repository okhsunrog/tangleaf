package dev.okhsunrog.mobile_system

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class EinkWiseSettingTest {
    /** The service: a write reaches the stored copy at once and the enforced one at the reboot. */
    private class Service(var stored: Boolean? = true, var active: Boolean? = true) {
        val writes = mutableListOf<Boolean>()
        val setting = EinkWiseSetting(
            readStored = { stored },
            readActive = { active },
            write = { stored = it; writes += it },
        )

        fun reboot() {
            active = stored
        }
    }

    @Test
    fun `turning it off is stored at once and enforced after the reboot`() {
        val service = Service()
        val pending = service.setting.set(false)
        assertEquals(EinkWiseSetting.State(available = true, enabled = false, active = true), pending)
        service.reboot()
        assertEquals(EinkWiseSetting.State(available = true, enabled = false, active = false), service.setting.state())
    }

    @Test
    fun `asking for what is already stored writes nothing`() {
        val service = Service(stored = false, active = false)
        service.setting.set(false)
        assertTrue(service.writes.isEmpty())
    }

    @Test
    fun `changing it back before the reboot restores the stored value`() {
        val service = Service()
        service.setting.set(false)
        val state = service.setting.set(true)
        assertEquals(listOf(false, true), service.writes)
        assertEquals(EinkWiseSetting.State(available = true, enabled = true, active = true), state)
    }

    @Test
    fun `a device without the service reports it unavailable and is left alone`() {
        val service = Service(stored = null, active = null)
        val state = service.setting.set(false)
        assertFalse(state.available)
        assertTrue(service.writes.isEmpty())
    }
}
