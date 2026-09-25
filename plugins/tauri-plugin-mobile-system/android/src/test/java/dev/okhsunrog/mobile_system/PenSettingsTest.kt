package dev.okhsunrog.mobile_system

import org.junit.Assert.*
import org.junit.Test

class PenSettingsTest {
    @Test fun thePenToolWithThePixelEraserSendsStocksWidthAndItsOwnStyleLast() {
        val calls = penSettingsCalls(
            fastLasso = false,
            interaction = false,
            eraser = false,
            eraserType = "pixel",
            eraserPx = eraserPx(500, 300),
            penPx = 5.2f,
        )
        assertEquals(
            listOf(
                PenCall.StrokeParameters(5, listOf(5f)),
                PenCall.StrokeParameters(8, listOf(59.05512f, 0.5f, 0.1f)),
                PenCall.StrokeWidth(5.2f),
                PenCall.StrokeColor(-16777216),
                PenCall.StrokeStyle(1),
                PenCall.EraserTrace(true, 8),
                PenCall.BrushTrace(true),
            ),
            calls,
        )
        // Stock's recorded payload for a 5.00 mm pixel eraser, bit for bit.
        val width = (calls[1] as PenCall.StrokeParameters).values[0]
        assertEquals(1114388593, java.lang.Float.floatToIntBits(width))
    }

    @Test fun theLassoToolKeepsItsDashAndNeverLetsTheEraserEndTouchTheStyleSlot() {
        val calls = penSettingsCalls(
            fastLasso = true,
            interaction = true,
            eraser = false,
            eraserType = "pixel",
            eraserPx = eraserPx(500, 300),
            penPx = 5.2f,
        )
        assertEquals(PenCall.StrokeParameters(5, listOf(5f)), calls.first())
        assertEquals(
            listOf(PenCall.StrokeWidth(5f), PenCall.StrokeColor(-16777216), PenCall.StrokeStyle(5)),
            calls.subList(2, 5),
        )
        assertTrue(PenCall.EraserTrace(false, 8) in calls)
    }

    @Test fun theStrokeEraserSendsItsNarrowWidth() {
        val calls = penSettingsCalls(
            fastLasso = false,
            interaction = false,
            eraser = false,
            eraserType = "stroke",
            eraserPx = eraserPx(50, 300),
            penPx = 5.2f,
        )
        assertEquals(PenCall.StrokeParameters(8, listOf(5.905512f, 0.5f, 0.1f)), calls[1])
        assertTrue(PenCall.EraserTrace(true, 8) in calls)
    }

    /** Stock's pen with the area eraser selected: the eraser end draws the dash, `[1, 5]`. */
    @Test fun thePenToolWithTheLassoTypeLetsTheEraserEndDrawTheDash() {
        val calls = penSettingsCalls(
            fastLasso = false,
            interaction = false,
            eraser = false,
            eraserType = "lasso",
            eraserPx = eraserPx(50, 300),
            penPx = 5.2f,
        )
        assertEquals(
            listOf(
                PenCall.StrokeParameters(5, listOf(5f)),
                PenCall.StrokeParameters(8, listOf(5.905512f, 0.5f, 0.1f)),
                PenCall.StrokeWidth(5.2f),
                PenCall.StrokeColor(-16777216),
                PenCall.StrokeStyle(1),
                PenCall.EraserTrace(true, 5),
                PenCall.BrushTrace(true),
            ),
            calls,
        )
    }

    /** Stock on selecting its eraser (`A7:19-45`): style 8, the soft eraser's parameters, no width. */
    @Test fun theEraserToolMakesTheSoftEraserItsStyleAndSendsNoWidth() {
        val calls = penSettingsCalls(
            fastLasso = false,
            interaction = false,
            eraser = true,
            eraserType = "pixel",
            eraserPx = eraserPx(500, 300),
            penPx = 5.2f,
        )
        assertEquals(
            listOf(
                PenCall.StrokeParameters(5, listOf(5f)),
                PenCall.StrokeParameters(8, listOf(59.05512f, 0.5f, 0.1f)),
                PenCall.StrokeColor(-16777216),
                PenCall.StrokeStyle(8),
                PenCall.EraserTrace(true, 8),
                PenCall.BrushTrace(true),
            ),
            calls,
        )
        val stroke = penSettingsCalls(false, false, true, "stroke", eraserPx(50, 300), 5.2f)
        assertEquals(PenCall.StrokeParameters(8, listOf(5.905512f, 0.5f, 0.1f)), stroke[1])
        assertTrue(PenCall.StrokeStyle(8) in stroke)
        assertTrue(stroke.none { it is PenCall.StrokeWidth })
    }

    /**
     * Stock on choosing its lasso eraser (`A7:241-252`): style 5, the dash parameters, the soft
     * eraser's at 0.50 mm, `[1, 5]`, and no width, so the dash keeps the previous tool's width.
     */
    @Test fun theEraserToolWithTheLassoTypeMakesTheDashItsStyleAndSendsNoWidth() {
        val calls = penSettingsCalls(
            fastLasso = false,
            interaction = false,
            eraser = true,
            eraserType = "lasso",
            eraserPx = eraserPx(50, 300),
            penPx = 5.2f,
        )
        assertEquals(
            listOf(
                PenCall.StrokeParameters(5, listOf(5f)),
                PenCall.StrokeParameters(8, listOf(5.905512f, 0.5f, 0.1f)),
                PenCall.StrokeColor(-16777216),
                PenCall.StrokeStyle(5),
                PenCall.EraserTrace(true, 5),
                PenCall.BrushTrace(true),
            ),
            calls,
        )
        // Stock's recorded payload for the lasso eraser's fixed 0.50 mm, bit for bit.
        val width = (calls[1] as PenCall.StrokeParameters).values[0]
        assertEquals(1086126580, java.lang.Float.floatToIntBits(width))
    }

    /** The lasso tool keeps its own dash and width, and the eraser end stays off, whatever the type. */
    @Test fun theLassoToolWithTheLassoTypeStillKeepsTheEraserEndOff() {
        val calls = penSettingsCalls(
            fastLasso = true,
            interaction = true,
            eraser = false,
            eraserType = "lasso",
            eraserPx = eraserPx(50, 300),
            penPx = 5.2f,
        )
        assertEquals(
            listOf(PenCall.StrokeWidth(5f), PenCall.StrokeColor(-16777216), PenCall.StrokeStyle(5)),
            calls.subList(2, 5),
        )
        assertTrue(PenCall.EraserTrace(false, 5) in calls)
        // The rectangle lasso draws nothing of its own and keeps the pen's style.
        val rectangle = penSettingsCalls(false, true, false, "lasso", eraserPx(50, 300), 5.2f)
        assertTrue(PenCall.StrokeStyle(1) in rectangle)
        assertTrue(PenCall.EraserTrace(false, 5) in rectangle)
    }

    /** Switching the eraser tool from the lasso type back to Pixel, and then to the pen. */
    @Test fun leavingTheLassoTypePutsTheSoftEraserAndThenThePenBack() {
        val lasso = penSettingsCalls(false, false, true, "lasso", eraserPx(50, 300), 5.2f)
        val pixel = penSettingsCalls(false, false, true, "pixel", eraserPx(500, 300), 5.2f)
        val pen = penSettingsCalls(false, false, false, "pixel", eraserPx(500, 300), 5.2f)
        assertTrue(PenCall.StrokeStyle(5) in lasso)
        assertTrue(PenCall.StrokeStyle(8) in pixel && PenCall.EraserTrace(true, 8) in pixel)
        assertEquals(PenCall.StrokeParameters(8, listOf(59.05512f, 0.5f, 0.1f)), pixel[1])
        assertTrue(PenCall.StrokeStyle(1) in pen && PenCall.StrokeWidth(5.2f) in pen)
    }

    /** Switching back from the eraser tool: the resume puts the pen's width and style 1 back. */
    @Test fun thePenAfterTheEraserToolGetsItsOwnStyleAndWidthBack() {
        val eraserTool = penSettingsCalls(false, false, true, "pixel", 59f, 5.2f)
        val pen = penSettingsCalls(false, false, false, "pixel", 59f, 5.2f)
        assertTrue(PenCall.StrokeStyle(8) in eraserTool)
        assertTrue(PenCall.StrokeStyle(1) in pen)
        assertTrue(PenCall.StrokeWidth(5.2f) in pen)
    }

    @Test fun everyResumeEndsWithTheFlagsAfterTheStyle() {
        for (type in listOf("pixel", "lasso")) {
            for ((fastLasso, eraser) in listOf(false to false, true to false, false to true)) {
                val calls = penSettingsCalls(fastLasso, fastLasso, eraser, type, 59f, 5f)
                val style = calls.indexOfFirst { it is PenCall.StrokeStyle }
                val eraserFlag = calls.indexOfFirst { it is PenCall.EraserTrace }
                assertTrue(style in 0 until eraserFlag)
                assertEquals(PenCall.BrushTrace(true), calls.last())
            }
        }
    }

    private data class TraceCase(
        val erasing: Boolean,
        val eraser: Boolean,
        val interaction: Boolean,
        val trace: Boolean,
    )

    @Test fun theFirmwareTracesEveryEraseButTheLassoTools() {
        val table = listOf(
            TraceCase(erasing = true, eraser = false, interaction = false, trace = true), // eraser end, pen
            TraceCase(erasing = false, eraser = true, interaction = false, trace = true), // eraser tool, tip
            TraceCase(erasing = true, eraser = true, interaction = false, trace = true), // its eraser end
            TraceCase(erasing = false, eraser = false, interaction = false, trace = false), // pen tip
            TraceCase(erasing = true, eraser = false, interaction = true, trace = false), // lasso tool, eraser end
            TraceCase(erasing = false, eraser = false, interaction = true, trace = false), // lasso tool, tip
        )
        for (case in table) {
            assertEquals(case.toString(), case.trace, eraseTrace(case.erasing, case.eraser, case.interaction))
        }
    }

    @Test fun theEraserToolRendersWhateverItsType() {
        // eraser, interaction, fastLasso, hasSelection -> renders
        assertTrue(toolRenders(eraser = true, interaction = false, fastLasso = false, hasSelection = false))
        assertTrue(toolRenders(eraser = true, interaction = false, fastLasso = false, hasSelection = true))
        // The pen always renders; the lasso tool only its free-form trace with nothing selected.
        assertTrue(toolRenders(eraser = false, interaction = false, fastLasso = false, hasSelection = true))
        assertTrue(toolRenders(eraser = false, interaction = true, fastLasso = true, hasSelection = false))
        assertFalse(toolRenders(eraser = false, interaction = true, fastLasso = true, hasSelection = true))
        assertFalse(toolRenders(eraser = false, interaction = true, fastLasso = false, hasSelection = false))
    }

    @Test fun theEraserEndDirtiesTheStyleSlotOnlyUnderAToolWithAnotherStyle() {
        assertEquals(PenStyle.SOFT_ERASER, toolStrokeStyle(eraser = true, fastLasso = false, eraserType = "pixel"))
        assertEquals(PenStyle.DASH, toolStrokeStyle(eraser = true, fastLasso = false, eraserType = "lasso"))
        assertEquals(PenStyle.FOUNTAIN, toolStrokeStyle(eraser = false, fastLasso = false, eraserType = "pixel"))
        assertEquals(PenStyle.FOUNTAIN, toolStrokeStyle(eraser = false, fastLasso = false, eraserType = "lasso"))
        assertEquals(PenStyle.DASH, toolStrokeStyle(eraser = false, fastLasso = true, eraserType = "pixel"))
        assertEquals(PenStyle.DASH, eraserEndStyle("lasso"))
        assertEquals(PenStyle.SOFT_ERASER, eraserEndStyle("stroke"))

        // The pen: the eraser end's painter is never the pen's, so the tip after it needs a re-send.
        assertTrue(eraserEndOverwritesStyle(eraser = false, fastLasso = false, eraserType = "pixel"))
        assertTrue(eraserEndOverwritesStyle(eraser = false, fastLasso = false, eraserType = "lasso"))
        // The eraser tool: its tip draws with the eraser end's own painter.
        assertFalse(eraserEndOverwritesStyle(eraser = true, fastLasso = false, eraserType = "pixel"))
        assertFalse(eraserEndOverwritesStyle(eraser = true, fastLasso = false, eraserType = "stroke"))
        assertFalse(eraserEndOverwritesStyle(eraser = true, fastLasso = false, eraserType = "lasso"))
    }

    @Test fun theEraserToolsPenUpRefreshNeverReconcilesInsideTheHandBack() {
        assertFalse(penUpRefreshReconciles(eraser = true, interaction = false))
        // The pen and the lasso tool keep their reconcile.
        assertTrue(penUpRefreshReconciles(eraser = false, interaction = false))
        assertTrue(penUpRefreshReconciles(eraser = false, interaction = true))
    }
}
