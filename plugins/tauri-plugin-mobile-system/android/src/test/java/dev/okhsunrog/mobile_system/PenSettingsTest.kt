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

    @Test fun theLassoTypeKeepsTheEraserEndOffUntilItsTraceExists() {
        val calls = penSettingsCalls(
            fastLasso = false,
            interaction = false,
            eraser = false,
            eraserType = "lasso",
            eraserPx = eraserPx(50, 300),
            penPx = 5.2f,
        )
        assertTrue(PenCall.EraserTrace(false, 5) in calls)
        assertEquals(PenCall.StrokeStyle(1), calls[4])
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

    @Test fun theEraserToolWithTheLassoTypeKeepsThePensSettingsForNow() {
        val calls = penSettingsCalls(
            fastLasso = false,
            interaction = false,
            eraser = true,
            eraserType = "lasso",
            eraserPx = eraserPx(50, 300),
            penPx = 5.2f,
        )
        assertEquals(
            penSettingsCalls(false, false, false, "lasso", eraserPx(50, 300), 5.2f),
            calls,
        )
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
        for ((fastLasso, eraser) in listOf(false to false, true to false, false to true)) {
            val calls = penSettingsCalls(fastLasso, fastLasso, eraser, "pixel", 59f, 5f)
            val style = calls.indexOfFirst { it is PenCall.StrokeStyle }
            val eraserFlag = calls.indexOfFirst { it is PenCall.EraserTrace }
            assertTrue(style in 0 until eraserFlag)
            assertEquals(PenCall.BrushTrace(true), calls.last())
        }
    }

    private data class TraceCase(
        val erasing: Boolean,
        val eraser: Boolean,
        val interaction: Boolean,
        val type: String,
        val trace: Boolean,
    )

    @Test fun theFirmwareTracesEveryEraseButTheLassoToolsAndTheLassoTypes() {
        val table = listOf(
            TraceCase(erasing = true, eraser = false, interaction = false, type = "pixel", trace = true), // eraser end, pen
            TraceCase(erasing = true, eraser = false, interaction = false, type = "stroke", trace = true),
            TraceCase(erasing = false, eraser = true, interaction = false, type = "pixel", trace = true), // eraser tool, tip
            TraceCase(erasing = true, eraser = true, interaction = false, type = "stroke", trace = true), // its eraser end
            TraceCase(erasing = false, eraser = false, interaction = false, type = "pixel", trace = false), // pen tip
            TraceCase(erasing = true, eraser = false, interaction = true, type = "pixel", trace = false), // lasso tool
            TraceCase(erasing = false, eraser = false, interaction = true, type = "pixel", trace = false),
            TraceCase(erasing = true, eraser = false, interaction = false, type = "lasso", trace = false), // step 4
            TraceCase(erasing = false, eraser = true, interaction = false, type = "lasso", trace = false),
        )
        for (case in table) {
            assertEquals(case.toString(), case.trace, eraseTrace(case.erasing, case.eraser, case.interaction, case.type))
        }
    }

    @Test fun theEraserToolRendersUnlessItsTypeIsTheLasso() {
        // eraser, type, interaction, fastLasso, hasSelection -> renders
        assertTrue(toolRenders(eraser = true, eraserType = "pixel", interaction = false, fastLasso = false, hasSelection = false))
        assertTrue(toolRenders(eraser = true, eraserType = "stroke", interaction = false, fastLasso = false, hasSelection = true))
        assertFalse(toolRenders(eraser = true, eraserType = "lasso", interaction = false, fastLasso = false, hasSelection = false))
        // The pen always renders; the lasso tool only its free-form trace with nothing selected.
        assertTrue(toolRenders(eraser = false, eraserType = "lasso", interaction = false, fastLasso = false, hasSelection = true))
        assertTrue(toolRenders(eraser = false, eraserType = "pixel", interaction = true, fastLasso = true, hasSelection = false))
        assertFalse(toolRenders(eraser = false, eraserType = "pixel", interaction = true, fastLasso = true, hasSelection = true))
        assertFalse(toolRenders(eraser = false, eraserType = "pixel", interaction = true, fastLasso = false, hasSelection = false))
    }

    @Test fun theEraserEndDirtiesTheStyleSlotOnlyUnderAToolWithAnotherStyle() {
        assertEquals(PenStyle.SOFT_ERASER, toolStrokeStyle(eraser = true, fastLasso = false, eraserType = "pixel"))
        assertEquals(PenStyle.FOUNTAIN, toolStrokeStyle(eraser = true, fastLasso = false, eraserType = "lasso"))
        assertEquals(PenStyle.FOUNTAIN, toolStrokeStyle(eraser = false, fastLasso = false, eraserType = "pixel"))
        assertEquals(PenStyle.DASH, toolStrokeStyle(eraser = false, fastLasso = true, eraserType = "pixel"))
        assertEquals(PenStyle.DASH, eraserEndStyle("lasso"))
        assertEquals(PenStyle.SOFT_ERASER, eraserEndStyle("stroke"))

        assertTrue(eraserEndOverwritesStyle(eraser = false, fastLasso = false, eraserType = "pixel"))
        assertFalse(eraserEndOverwritesStyle(eraser = true, fastLasso = false, eraserType = "pixel"))
        assertFalse(eraserEndOverwritesStyle(eraser = true, fastLasso = false, eraserType = "stroke"))
    }

    @Test fun theEraserToolsPenUpRefreshNeverReconcilesInsideTheHandBack() {
        assertFalse(penUpRefreshReconciles(eraser = true, interaction = false, eraserType = "pixel"))
        assertFalse(penUpRefreshReconciles(eraser = true, interaction = false, eraserType = "stroke"))
        // The lasso type keeps today's reconcile; so do the pen and the lasso tool.
        assertTrue(penUpRefreshReconciles(eraser = true, interaction = false, eraserType = "lasso"))
        assertTrue(penUpRefreshReconciles(eraser = false, interaction = false, eraserType = "pixel"))
        assertTrue(penUpRefreshReconciles(eraser = false, interaction = true, eraserType = "pixel"))
    }
}
