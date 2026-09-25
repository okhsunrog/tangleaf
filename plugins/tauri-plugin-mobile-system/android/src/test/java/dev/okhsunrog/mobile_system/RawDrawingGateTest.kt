package dev.okhsunrog.mobile_system

import org.junit.Assert.*
import org.junit.Test

private class RecordingSwitches : RawDrawingSwitches {
    val calls = mutableListOf<String>()
    override fun render(enabled: Boolean) { calls.add("render=$enabled") }
    override fun input(enabled: Boolean) { calls.add("input=$enabled") }
    override fun pushRects() { calls.add("rects") }
    override fun pushPen() { calls.add("pen") }
}

class RawDrawingGateTest {
    @Test fun pausingStopsRenderingBeforeTheReader() {
        val switches = RecordingSwitches()
        RawDrawingGate(switches).pause()
        assertEquals(listOf("render=false", "input=false"), switches.calls)
    }

    @Test fun resumingRePushesGeometryAndPenAndArmsInputBeforeRender() {
        val switches = RecordingSwitches()
        RawDrawingGate(switches).resume(render = true)
        assertEquals(listOf("rects", "pen", "input=true", "render=true"), switches.calls)
    }

    @Test fun aToolThatOnlyWantsRawPointsResumesWithoutFirmwareInk() {
        val switches = RecordingSwitches()
        RawDrawingGate(switches).resume(render = false)
        assertEquals(listOf("rects", "pen", "input=true", "render=false"), switches.calls)
    }

    @Test fun theEraseHandBackResumesRenderAloneWithTheFullReSend() {
        val switches = RecordingSwitches()
        RawDrawingGate(switches).resume(render = true, input = false)
        assertEquals(listOf("rects", "pen", "render=true"), switches.calls)
    }
}
