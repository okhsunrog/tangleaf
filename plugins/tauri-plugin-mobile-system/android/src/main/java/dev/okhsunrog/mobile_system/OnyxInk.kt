package dev.okhsunrog.mobile_system

import android.app.Activity
import android.graphics.Color
import android.graphics.Rect
import android.graphics.RectF
import android.os.Build
import android.os.SystemClock
import android.util.Log
import android.view.ViewTreeObserver
import android.webkit.WebView
import app.tauri.annotation.InvokeArg
import app.tauri.plugin.JSObject
import com.onyx.android.sdk.api.device.epd.EpdController
import com.onyx.android.sdk.api.device.epd.UpdateMode
import com.onyx.android.sdk.device.Device
import com.onyx.android.sdk.data.note.TouchPoint
import com.onyx.android.sdk.pen.EpdPenManager
import com.onyx.android.sdk.pen.RawInputCallback
import com.onyx.android.sdk.pen.TouchHelper
import com.onyx.android.sdk.pen.data.TouchPointList
import com.onyx.android.sdk.utils.ResManager
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.ceil
import kotlin.math.floor

@InvokeArg
class OnyxInkArgs {
    var session: String = ""
    var enabled: Boolean = false
    var left: Double = 0.0
    var top: Double = 0.0
    var width: Double = 0.0
    var height: Double = 0.0
    var clipTop: Double = 0.0
    var clipBottom: Double = 0.0
    var viewportWidth: Double = 0.0
    var strokeWidth: Double = 3.0
    var eraser: Boolean = false
    var interaction: Boolean = false
    var fastLasso: Boolean = false
    var hasSelection: Boolean = false
    var selectionLeft: Double = 0.0
    var selectionTop: Double = 0.0
    var selectionRight: Double = 0.0
    var selectionBottom: Double = 0.0
    /** DOM controls floating inside the sheet that a pen must tap rather than ink, in CSS pixels. */
    var overlayRects: List<OnyxOverlayRect> = emptyList()
}

@InvokeArg
class OnyxOverlayRect {
    var left: Double = 0.0
    var top: Double = 0.0
    var width: Double = 0.0
    var height: Double = 0.0
}

@InvokeArg
class OnyxFrameArgs {
    var partial: Boolean = false
    var left: Double = 0.0
    var top: Double = 0.0
    var right: Double = 0.0
    var bottom: Double = 0.0
    var session: String = ""
    var sequence: Long = 0
}

/** Vendor fast ink is transient. Completed points go to the ordinary, durable web canvas. */
internal class OnyxInk(
    private val activity: Activity,
    private val webView: WebView,
    private val displayMode: ViewDisplayMode,
    private val pauses: InkPauseRegistry,
    private val emit: (JSObject) -> Unit,
) : ViewTreeObserver.OnWindowFocusChangeListener {
    private var helper: TouchHelper? = null
    private var config: OnyxInkArgs? = null
    private var sheet = RectF()
    private var limit = Rect()
    private var overlays: List<InkRect> = emptyList()
    /** The pen landed on a floating control: this gesture inks nothing and reports nothing. */
    private var swallowed = false
    private var resumed = true
    private val gesture = InkGesturePairing()
    private var sequence = 0L
    private var generation = 0L
    private val frames = InkFrameFence()
    private var repaintCount = 0L
    /** Where the next reconcile has to cover, in view pixels: stock's dirty rect. */
    private val region = InkReconcileRegion()
    private val reconcile = InkReconcileGate()
    private var penUpRefreshCount = 0L
    private val qualityDamage = InkDamage()
    private var lastRepaint = Rect()
    private var lastRepaintWholeRegion = false
    private var repaintedPixels = 0L
    private var pendingFrame: Runnable? = null
    private var pendingObserver: ViewTreeObserver? = null
    private var fastPreview = false
    private var previewAt = 0L
    private var maxPressure = 4095f
    private var failed: String? = null
    private var imeVisible = false
    private var imeSource: String? = null
    private val refresh = Runnable { refreshFrame() }
    /** The pen-up refresh never came for this gesture; reconcile without it. */
    private val penUpWait = Runnable { if (reconcile.timedOut() != InkReconcileGate.Next.WAIT) refreshFrame() }
    private val gate = RawDrawingGate(object : RawDrawingSwitches {
        override fun render(enabled: Boolean) { setRender(enabled) }
        override fun input(enabled: Boolean) { helper?.setRawInputReaderEnable(enabled) }
        override fun pushRects() {
            // Single-region mode and the pen-up refresh interval, re-applied on every resume
            // exactly as stock Notes does in ResumeRawDrawingRequest (:108-114). Without the
            // region mode the firmware keeps one transient region per stroke; once that table
            // fills, whole rectangles of the panel stop taking new ink until the scribble state
            // is torn down.
            helper?.setSingleRegionMode()
            helper?.setPenUpRefreshTimeMs(PEN_UP_REFRESH_MS.toInt())
            clearLatchedExcludes()
            helper?.setLimitRect(limit, NO_EXCLUDES)
        }
        override fun resetDefaults() { helper?.resetPenDefaultRawDrawing() }
    })
    // Resuming into the tail of an IME teardown leaves ghost ink; stock Notes waits too.
    private val resumeGate = Runnable { resume() }
    private val eraserRenderGate = InkEraserRenderGate(
        pause = { setRender(false) },
        resume = { restoreToolRendering() },
    )

    private var lastRepaintModeRaw: Int? = null
    private val repaintMode = InkRepaintMode(
        enter = {
            displayMode.set(DisplayModeStack.Layer.TRANSIENT, UpdateMode.HAND_WRITING_REPAINT_MODE)
            lastRepaintModeRaw = displayMode.readRaw()
        },
        // Back to whatever still owns the view: the editor's fast mode, or the profile under it.
        leave = { displayMode.clear(DisplayModeStack.Layer.TRANSIENT) },
    )
    private var fastModeAccepted: Boolean? = null
    private var fastModeRequests = 0L
    private var qualityRestores = 0L
    private val displayPolicy = InkRefreshPolicy(
        enter = {
            fastModeAccepted = EpdController.applyTransientUpdate(UpdateMode.ANIMATION_QUALITY)
            fastModeRequests++
            Log.d("OnyxInk", "transient animation quality requested: $fastModeAccepted")
        },
        leave = {
            // The EpdController wrapper discards this return value; retain it for diagnosis.
            val accepted = Device.currentDevice().clearTransientUpdate(false)
            qualityRestores++
            Log.d("OnyxInk", "transient mode cleared: $accepted")
        },
    )
    private val settleDisplay = Runnable {
        if (!gesture.drawing && displayPolicy.settle(SystemClock.uptimeMillis())) {
            val dirty = qualityDamage.take()
            // Repaint even if the last content revision was already presented in fast mode.
            config?.let { args -> commit(OnyxFrameArgs().apply {
                session = args.session
                sequence = this@OnyxInk.sequence
                partial = true
                if (dirty != null) {
                    left = dirty.left; top = dirty.top; right = dirty.right; bottom = dirty.bottom
                }
            }) }
        }
    }

    companion object {
        /** Dash length and gap, and the line width, of the firmware selection trace. */
        private const val LASSO_DASH = 5f

        /**
         * The handwriting region is never punched through. Floating controls are handled per
         * gesture instead, so a lasso trace crossing one is still drawn whole — the stock Notes
         * selection popup registers no exclude rect either.
         */
        private val NO_EXCLUDES = emptyList<Rect>()

        /**
         * The exclusion table lives in the SurfaceFlinger process, keyed by nothing at all — the
         * transaction carries no pid, no window and no surface, and nothing reaps it when the
         * caller dies. A rect pushed by an earlier run of this app therefore outlives it and keeps
         * swallowing pen input until the device reboots.
         *
         * The receiving side clears the table only when the payload is empty
         * (`PenManager::setExcludeRegion`, which takes its `n <= 0` branch); a zero-area rectangle
         * is *stored*, not treated as a reset, and then still excludes a box at the origin because
         * the hit test inflates every exclusion by half the stroke width. So the reset is an empty
         * array, and the null view keeps the coordinate translation from running over it.
         *
         * `RawInputReader.setExcludeRect` cannot express this — it returns early on an empty list —
         * but the app-side reader has no such guard, so `setLimitRect(limit, NO_EXCLUDES)` already
         * wipes its half of the state on every resume.
         */
        private val EXCLUDE_RESET = emptyArray<Rect>()
        private const val CLOSE_REFRESH_DELAY_MS = 300L
        fun supported(): Boolean = Build.MANUFACTURER.equals("ONYX", true)

        /**
         * `RawInputReader`'s default pen-up refresh interval, re-applied on every resume the way
         * `ResumeRawDrawingRequest` does. The SDK starts this timer at the last point of a
         * non-erasing stroke and then reports the stroke union through `onPenUpRefresh`.
         */
        const val PEN_UP_REFRESH_MS = 500L

        /**
         * How long the reconcile waits for that callback once the canvas frame is ready. Only a
         * firmware that does not deliver it at all should ever hit this.
         */
        const val PEN_UP_WAIT_MS = PEN_UP_REFRESH_MS + 250

        /** The compositor acknowledgement is a readiness signal, not a submitted frame. */
        private const val FRAME_DELAY_MS = 120L

        /**
         * Stock Notes' `DELAY_ENABLE_RAW_DRAWING_MILLS`
         * (`RawPenArgs.java:69`: `isColorDevice() ? 500 : 150`). Resuming into the tail of whatever
         * covered the panel leaves ghost ink, and a colour panel needs the longer wait.
         */
        val RESUME_DELAY_MS: Long
            get() = if (colorPanel()) 500L else 150L

        /**
         * `DeviceInfoUtil.isColorDevice()` is `Device.currentDevice().getColorType() > 0`. A probe
         * that fails answers "colour", which only ever means the slower, safer resume delay.
         */
        private var colorPanelProbe: Boolean? = null

        private fun colorPanel(): Boolean = colorPanelProbe ?: runCatching { Device.currentDevice().colorType > 0 }
            .onFailure { Log.d("OnyxInk", "no colour type reported: ${it.message}") }
            .getOrDefault(true).also { colorPanelProbe = it }

        const val IME_REASON = "ime"
    }

    init {
        // Vendor firmware APIs are hidden from apps targeting recent Android versions.
        // Initialize before any EpdController/Device static lookup, including cleanup calls.
        check(VendorAccess.ensure()) { "BOOX firmware drawing APIs are unavailable" }
        // A session killed mid-gesture leaves the panel in transient mode, and
        // nothing else ever clears it. Start from a known state.
        runCatching { Device.currentDevice().clearTransientUpdate(false) }
            .onFailure { Log.d("OnyxInk", "no transient mode to clear: ${it.message}") }
        // Nothing arms the capacitive-panel cutout any more (see resetPalm), but a build that
        // still did may have died holding one. This is the only place it is touched.
        runCatching { resetPalm() }
            .onFailure { Log.d("OnyxInk", "no palm region to clear: ${it.message}") }
        webView.viewTreeObserver.addOnWindowFocusChangeListener(this)
    }

    /** Reported by the plugin, which owns the one insets listener the WebView can have. */
    fun imeChanged(visible: Boolean) {
        if (visible == imeVisible) return
        imeVisible = visible
        Log.d("OnyxInk", "ime visible=$visible")
        if (visible) imeSource = "insets"
        if (visible) pauses.pause(IME_REASON) else pauses.resume(IME_REASON)
        pauseStateChanged()
    }

    /**
     * A reason was added or dropped. This is stock's `InvalidateScreenAction`: pause render and
     * input, invalidate the view so the app's own content covers whatever the firmware left, and
     * resume after `DELAY_ENABLE_RAW_DRAWING_MILLS`. Pausing is immediate — the keyboard is
     * already coming up — and the resume is cancelled if something pauses again meanwhile.
     */
    fun pauseStateChanged() {
        webView.removeCallbacks(resumeGate)
        // Keep the editor's display mode: the sheet is still on screen, only the pen is down.
        if (pauses.isPaused) pause(releaseDisplay = false)
        else webView.postDelayed(resumeGate, RESUME_DELAY_MS)
    }

    fun configure(args: OnyxInkArgs): JSObject {
        if (!args.enabled) {
            // Ignore cleanup from a sheet which has already been replaced.
            if (config?.session == args.session) close()
            return status()
        }
        require(args.session.length in 1..128)
        require(listOf(args.left, args.top, args.width, args.height, args.clipTop,
            args.clipBottom, args.viewportWidth, args.strokeWidth, args.selectionLeft, args.selectionTop,
            args.selectionRight, args.selectionBottom).all { it.isFinite() })
        require(args.width > 0 && args.height > 0 && args.viewportWidth > 0)
        require(args.strokeWidth in 0.1..20.0)
        // A transient SDK failure — a hidden-API hiccup, a digitizer that was
        // not ready yet — used to disable fast ink for the rest of the process.
        // The message survives for status(); the gate does not.
        failed = null
        check(Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && webView.isHardwareAccelerated) {
            "BOOX ink requires Android 10 or later with hardware rendering"
        }
        if (config?.session != args.session) {
            close()
        }
        // The overlay rects (the floating selection menu) only feed the JS-side hit test; they
        // never reach the firmware region. Updating just them must not restart raw drawing — a
        // pause would wipe the transient lasso trace mid-gesture, which showed up as segments
        // missing from the selection outline exactly where the menu had been.
        val current = config
        if (current != null && current.session == args.session && onlyOverlaysChanged(current, args)) {
            config = args
            val scaleOnly = webView.width / args.viewportWidth
            overlays = inkOverlayRects(args.overlayRects, scaleOnly, InkRect(limit.left, limit.top, limit.right, limit.bottom))
            // A floating control appearing over the sheet still draws over the panel, so the next
            // reconcile owes the whole region even though the pen was never paused for it.
            region.invalidateAll()
            return status()
        }
        pause(releaseDisplay = false, tearDown = false)
        config = args
        // CSS pixels may differ from Android density because BOOX has per-app DPI settings.
        val scale = webView.width / args.viewportWidth
        val previousSheet = RectF(sheet)
        val previousLimit = Rect(limit)
        val previousOverlays = overlays
        sheet = RectF((args.left * scale).toFloat(), (args.top * scale).toFloat(),
            ((args.left + args.width) * scale).toFloat(), ((args.top + args.height) * scale).toFloat())
        limit = Rect(floor(sheet.left).toInt(), ceil(maxOf(sheet.top.toDouble(), args.clipTop * scale)).toInt(),
            ceil(sheet.right).toInt(), floor(minOf(sheet.bottom.toDouble(), args.clipBottom * scale)).toInt())
        if (!limit.intersect(0, 0, webView.width, webView.height)) limit.setEmpty()
        overlays = inkOverlayRects(args.overlayRects, scale, InkRect(limit.left, limit.top, limit.right, limit.bottom))
        try {
            if (helper == null) {
                ResManager.init(activity.applicationContext)
                check(EpdController.getTouchWidth() > 0 && EpdController.getTouchHeight() > 0) {
                    "BOOX firmware did not expose the digitizer coordinate range"
                }
                maxPressure = EpdController.getMaxTouchPressure().takeIf { it > 0 } ?: 4095f
                // Experiment (2026-09-07): the stock Notes app creates its helper with
                // SF|APP touch render; with SF alone a finger touch inside the region
                // appeared to trigger a firmware refresh. Revert if pen rendering regresses.
                helper = TouchHelper.create(
                    webView,
                    TouchHelper.FEATURE_SF_TOUCH_RENDER or TouchHelper.FEATURE_APP_TOUCH_RENDER,
                    callback(generation),
                    false,
                )
                // The SDK's pen-up refresh drives the reconcile, as it does in stock Notes. It is
                // enabled by default in RawInputReader and stock never touches the (deprecated)
                // setter; only the interval is re-applied, here and on every resume.
                helper!!.setPenUpRefreshTimeMs(PEN_UP_REFRESH_MS.toInt())
                helper!!.setPostInputEvent(false)
                helper!!.setHostViewScrollListenerEnabled(false)
                helper!!.setSingleRegionMode()
                helper!!.setLimitRect(limit, NO_EXCLUDES).openRawDrawing()
                helper!!.setEraserRawDrawingEnabled(false, 0)
            }
            if (args.fastLasso) {
                // The firmware draws the selection trace as a dashed line only when the dash
                // pattern is configured on the device first: a single-element gap/length array,
                // black, at the driver's standard width (Notate's verified recipe, onyx.md §2).
                Device.currentDevice().setStrokeParameters(TouchHelper.STROKE_STYLE_DASH, floatArrayOf(LASSO_DASH))
                helper!!.setLimitRect(limit, NO_EXCLUDES)
                    .setStrokeWidth(LASSO_DASH)
                    .setStrokeColor(Color.BLACK)
                    .setStrokeStyle(TouchHelper.STROKE_STYLE_DASH)
            } else {
                helper!!.setLimitRect(limit, NO_EXCLUDES)
                    .setStrokeWidth((args.strokeWidth * sheet.width() / 1000).toFloat())
                    .setStrokeColor(Color.BLACK)
                    .setStrokeStyle(TouchHelper.STROKE_STYLE_FOUNTAIN)
            }
            resume()
            if (sheet != previousSheet || limit != previousLimit || overlays != previousOverlays) commit(OnyxFrameArgs().apply {
                session = args.session
                sequence = this@OnyxInk.sequence
            })
            check(helper!!.isRawDrawingCreated) { "Pen SDK did not create a drawing session" }
            return status()
        } catch (error: Throwable) {
            failed = error.message ?: error.javaClass.simpleName
            close()
            throw error
        }
    }

    /** True when two configs differ only in their overlay rects (sheet geometry unchanged). */
    private fun onlyOverlaysChanged(a: OnyxInkArgs, b: OnyxInkArgs): Boolean =
        a.enabled == b.enabled &&
            a.left == b.left && a.top == b.top && a.width == b.width && a.height == b.height &&
            a.clipTop == b.clipTop && a.clipBottom == b.clipBottom &&
            a.viewportWidth == b.viewportWidth && a.strokeWidth == b.strokeWidth &&
            a.eraser == b.eraser && a.interaction == b.interaction && a.fastLasso == b.fastLasso &&
            a.hasSelection == b.hasSelection &&
            a.selectionLeft == b.selectionLeft && a.selectionTop == b.selectionTop &&
            a.selectionRight == b.selectionRight && a.selectionBottom == b.selectionBottom &&
            a.overlayRects != b.overlayRects

    /** Diagnostics only: drive one firmware path at a time while a dead area is on screen. */
    fun debugRepaint(kind: String): String {
        if (limit.isEmpty) return "limit empty"
        return runCatching {
            when (kind) {
                "handwriting" -> { EpdController.handwritingRepaint(webView, limit); "handwritingRepaint $limit" }
                "invalidateGu" -> { EpdController.invalidate(webView, UpdateMode.GU); "invalidate GU" }
                "toggleRaw" -> { helper?.setRawDrawingRenderEnabled(false); helper?.setRawDrawingRenderEnabled(true); "render off/on" }
                "toggleInput" -> { helper?.setRawInputReaderEnable(false); helper?.setRawInputReaderEnable(true); "input off/on" }
                "clearExcludes" -> {
                    helper?.setExcludeRect(NO_EXCLUDES)
                    helper?.setLimitRect(limit, NO_EXCLUDES)
                    "excludes cleared, limit=$limit"
                }
                "excludeReset" -> { clearLatchedExcludes(); "vendor exclude reset" }
                "sessionCycle" -> { cycleScribbleSession(); "scribble session cycled" }
                "regionMode" -> { helper?.setSingleRegionMode(); "single region mode" }
                "leaveScribble" -> { EpdController.leaveScribbleMode(webView); "leaveScribbleMode" }
                else -> "unknown"
            }
        }.getOrElse { "error: ${it.message}" }
    }

    fun status(): JSObject = JSObject().apply {
        put("available", failed == null)
        put("active", helper?.isRawDrawingInputEnabled == true)
        put("error", failed)
        put("repaintCount", repaintCount)
        put("lastRepaint", lastRepaint.toShortString())
        put("lastRepaintWholeRegion", lastRepaintWholeRegion)
        put("wholeRegionPending", region.wholeRegionPending)
        put("penUpRefreshSeen", reconcile.penUpRefreshSeen)
        put("penUpRefreshCount", penUpRefreshCount)
        put("colorPanel", colorPanel())
        put("resumeDelayMs", RESUME_DELAY_MS)
        put("repaintModeActive", repaintMode.active)
        put("eraserRenderPaused", eraserRenderGate.active)
        put("lastRepaintModeRaw", lastRepaintModeRaw)
        put("repaintedPixels", repaintedPixels)
        put("visibleCanvasPixels", limit.width().toLong() * limit.height())
        put("qualityModeOwned", displayMode.isSet(DisplayModeStack.Layer.SESSION))
        put("viewUpdateMode", displayMode.readMode()?.name)
        put("viewUpdateModeRaw", displayMode.readRaw())
        put("requestedViewUpdateMode", displayMode.effective?.name)
        put("previousViewMode", displayMode.previousMode?.name)
        put("previousViewModeRaw", displayMode.previousRaw)
        put("fastModeRequested", displayPolicy.fastRequested)
        put("fastModeAccepted", fastModeAccepted)
        put("fastModeRequests", fastModeRequests)
        put("qualityRestores", qualityRestores)
        put("paused", JSONArray(pauses.reasons()))
        put("imeVisible", imeVisible)
        put("imeSource", imeSource)
    }

    fun commit(args: OnyxFrameArgs) {
        if (config?.session != args.session) return
        if (args.partial) addDamage(args.left, args.top, args.right, args.bottom)
        else addDamage(0.0, 0.0, 1000.0, 1400.0)
        cancelFrameSubmission()
        val revision = frames.request()
        // This only guarantees readiness for the next WebView draw, not a submitted frame.
        webView.postVisualStateCallback(revision, object : WebView.VisualStateCallback() {
            override fun onComplete(requestId: Long) {
                if (config?.session != args.session || !frames.ready(requestId, args.sequence)) return
                webView.removeCallbacks(refresh)
                webView.removeCallbacks(penUpWait)
                // Stock reconciles when the SDK's pen-up timer and its own bitmap flush have both
                // happened, whichever is last. This is our half of that rendezvous.
                when (reconcile.canvasReady()) {
                    InkReconcileGate.Next.NOW -> webView.post(refresh)
                    InkReconcileGate.Next.SOON -> webView.postDelayed(refresh, FRAME_DELAY_MS)
                    InkReconcileGate.Next.WAIT -> webView.postDelayed(penUpWait, PEN_UP_WAIT_MS)
                }
            }
        })
    }

    /**
     * The SDK's pen-up timer, ~500 ms after the last point of a non-erasing stroke. Its rectangle
     * is this stroke's bounds expanded by the stroke width and unioned with the previous stroke's
     * (`RawInputReader.i()`), which is exactly the region stock reconciles.
     */
    private fun penUpRefresh(rect: RectF) {
        if (config == null || !rect.left.isFinite() || !rect.top.isFinite() ||
            !rect.right.isFinite() || !rect.bottom.isFinite()
        ) return
        penUpRefreshCount++
        region.add(rect.left.toDouble(), rect.top.toDouble(), rect.right.toDouble(), rect.bottom.toDouble())
        if (reconcile.penUpRefresh() == InkReconcileGate.Next.NOW) {
            webView.removeCallbacks(refresh)
            webView.removeCallbacks(penUpWait)
            refreshFrame()
        }
    }

    /**
     * Stock's `GrayscaleRefreshAction`, and nothing else: mark the view's default update mode as
     * `HAND_WRITING_REPAINT_MODE`, draw one ordinary frame, reset the mode once that frame is on
     * screen (its `doFinally`). There is no `handwritingRepaint`, no `refreshScreenRegion`, no pen
     * state transition and no sleep anywhere in the stock ink path — the mode-marked frame *is*
     * the mechanism that hands the firmware layer back to the app.
     */
    private fun refreshFrame() {
        if (!canPresent() || pendingFrame != null) return
        val observer = webView.viewTreeObserver
        if (!observer.isAlive) return
        val area = region.take(InkBounds(
            limit.left.toDouble(), limit.top.toDouble(), limit.right.toDouble(), limit.bottom.toDouble(),
        )) ?: return
        reconcile.reconciled()
        val submission = frames.submission()
        val submitted = Runnable {
            // Frame callbacks may run off the UI thread. Recheck pen and lifecycle state there.
            webView.post {
                if (!frames.isCurrent(submission)) return@post
                pendingFrame = null
                pendingObserver = null
                var presented = false
                try {
                    if (!canPresent() || !frames.present(submission, sequence, gesture.drawing)) return@post
                    presented = true
                    lastRepaint = Rect(
                        floor(area.bounds.left).toInt(), floor(area.bounds.top).toInt(),
                        ceil(area.bounds.right).toInt(), ceil(area.bounds.bottom).toInt())
                    lastRepaintWholeRegion = area.wholeRegion
                    repaintedPixels += lastRepaint.width().toLong() * lastRepaint.height()
                    repaintCount++
                    // The canonical buffer is on screen; the eraser's pen-render pause can go.
                    eraserRenderGate.framePresented()
                } finally {
                    // A panel keeps whatever was last pushed to it. A region consumed by a frame
                    // that never reached the panel would leave that area showing an older image
                    // for good — the rectangular holes in dense ink — so it goes back on the pile.
                    if (!presented) region.putBack(area)
                    repaintMode.release()
                }
            }
        }
        pendingFrame = submitted
        pendingObserver = observer
        // Stock Notes marks the actual submitted buffer as a handwriting repaint.
        // A later handwritingRepaint call alone does not remove fast ink on this firmware.
        repaintMode.acquire()
        observer.registerFrameCommitCallback(submitted)
        // Stock invalidates its whole editor view — which *is* its writing region. Ours is the
        // whole screen, so this covers more; that only helps a region the firmware latched, and
        // the mode is reset the moment the frame lands.
        webView.invalidate()
    }

    private fun canPresent(): Boolean =
        helper != null && frames.canSubmit(sequence, gesture.drawing) &&
            resumed && webView.hasWindowFocus() && !pauses.isPaused && !limit.isEmpty

    private fun cancelFrameSubmission() {
        repaintMode.release()
        frames.cancelSubmission()
        val callback = pendingFrame
        val observer = pendingObserver
        if (callback != null && observer?.isAlive == true) observer.unregisterFrameCommitCallback(callback)
        pendingFrame = null
        pendingObserver = null
    }

    fun onPause() { resumed = false; pause() }
    fun onResume() { resumed = true; resume() }
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        webView.removeCallbacks(resumeGate)
        // Regaining focus is the same situation as a dialog closing: whatever covered the panel is
        // still being torn down, and resuming into that leaves ghost ink. Every other resume path
        // waits DELAY_ENABLE_RAW_DRAWING_MILLS; this one used to be the exception.
        if (hasFocus) webView.postDelayed(resumeGate, RESUME_DELAY_MS) else pause()
    }

    private fun resume() {
        if (!resumed || !webView.hasWindowFocus() || pauses.isPaused || config == null || limit.isEmpty) return
        webView.removeCallbacks(resumeGate)
        val acquiredQuality = !displayMode.isSet(DisplayModeStack.Layer.SESSION)
        if (acquiredQuality) {
            displayMode.set(DisplayModeStack.Layer.SESSION, UpdateMode.GU)
            Log.d("OnyxInk", "view quality mode: ${displayMode.readMode()}, previous: ${displayMode.previousMode}")
        }
        gate.resume(toolRendering())
        if (acquiredQuality) config?.let { args -> commit(OnyxFrameArgs().apply {
            session = args.session
            sequence = this@OnyxInk.sequence
        }) }
    }

    /**
     * @param tearDown whether to rebuild the SurfaceFlinger ink session as well. A real pause — a
     * dialog, the keyboard, losing focus — is the moment to do it. A reconfigure is not: the pen is
     * only being re-armed over new geometry, the firmware's own geometry path likewise skips its
     * repaint, and `configure` runs on every scroll frame, so tearing the session down there would
     * put a schema rebuild on a hot path.
     */
    private fun pause(releaseDisplay: Boolean = true, tearDown: Boolean = true) {
        if (releaseDisplay || gesture.drawing) releaseDisplayMode()
        webView.removeCallbacks(refresh)
        webView.removeCallbacks(penUpWait)
        webView.removeCallbacks(resumeGate)
        frames.request() // Invalidate visual/frame callbacks from the old geometry or lifecycle.
        cancelFrameSubmission()
        gate.pause()
        if (tearDown) cycleScribbleSession()
        // Whatever pauses the pen — a dialog, a menu, the keyboard, a tool or page change — also
        // draws over the panel. Stock clears `isEnabledPenDirtyRect` for exactly that, so the
        // first reconcile afterwards covers the whole writing region instead of a stroke union.
        region.invalidateAll()
        reconcile.reset()
        // Stock's InvalidateViewWithPenControlAction: with the pen layer down, put the app's own
        // content back on the panel before anything is allowed to ink over it again.
        webView.invalidate()
        eraserRenderGate.reset()
        swallowed = false
        if (gesture.ended()) send("cancel")
    }

    /** Whether the active tool wants firmware ink rather than only its raw points. */
    private fun toolRendering(): Boolean = config?.eraser == false &&
        (config?.interaction == false || (config?.fastLasso == true && config?.hasSelection == false))

    private fun restoreToolRendering() {
        setRender(!pauses.isPaused && toolRendering())
    }

    private fun setRender(enabled: Boolean) {
        helper?.setRawDrawingRenderEnabled(enabled)
    }

    /**
     * Drop whatever exclusion is latched in SurfaceFlinger. See [EXCLUDE_RESET]: this is the
     * firmware's own reset, and the null view is the part that makes it one.
     */
    private fun clearLatchedExcludes() {
        runCatching { EpdController.setScreenHandWritingRegionExclude(null, EXCLUDE_RESET) }
            .onFailure { Log.w("OnyxInk", "exclude reset failed: ${it.message}") }
    }

    /**
     * Tear down and rebuild the ink session on the SurfaceFlinger side.
     *
     * Two things get stuck independently. An exclusion stops the pen from inking, and
     * [clearLatchedExcludes] is what clears that. Separately SurfaceFlinger can stay in the
     * handwriting schema with the app marked as not drawing, and there it composites only ink and
     * ignores our layers — that is the half where our own content stops appearing. Switching the
     * schema while in handwriting mode is refused outright; the only way out is the pen state
     * going through PAUSE, which is also what `leaveScribbleMode` and the pen-state pair below
     * emit. Nothing else reaches it: a repaint request is issued *inside* the wedged session, and
     * an ordinary invalidate produces exactly the kind of frame that session declines to post.
     *
     * Called through [EpdController] rather than [TouchHelper] on purpose. `setRawDrawingRenderEnabled`
     * skips the call when the flag already holds the requested value, and with the eraser or a
     * lasso active it is already false — so the cycle that is supposed to heal the panel would
     * emit nothing at all.
     */
    private fun cycleScribbleSession() {
        runCatching {
            EpdController.leaveScribbleMode(webView)
            EpdController.setScreenHandWritingPenState(webView, EpdPenManager.PEN_PAUSE)
        }.onFailure { Log.w("OnyxInk", "scribble teardown failed: ${it.message}") }
    }

    private fun releaseDisplayMode() {
        repaintMode.release()
        webView.removeCallbacks(settleDisplay)
        displayPolicy.reset()
        qualityDamage.take()
        // The display profile may still hold the layer below; the stack restores the raw mode
        // only once nothing is left above it.
        displayMode.clear(DisplayModeStack.Layer.SESSION)
    }

    /** Sheet coordinates in, view pixels out: the reconcile region lives in the panel's space. */
    private fun addDamage(left: Double, top: Double, right: Double, bottom: Double) {
        if (displayPolicy.fastRequested) qualityDamage.add(left, top, right, bottom)
        if (sheet.isEmpty) return
        region.add(
            sheet.left + left.coerceIn(0.0, 1000.0) * sheet.width() / 1000,
            sheet.top + top.coerceIn(0.0, 1400.0) * sheet.height() / 1400,
            sheet.left + right.coerceIn(0.0, 1000.0) * sheet.width() / 1000,
            sheet.top + bottom.coerceIn(0.0, 1400.0) * sheet.height() / 1400,
        )
    }

    private fun markNativePoint(point: TouchPoint) {
        if (config == null || sheet.isEmpty || !point.x.isFinite() || !point.y.isFinite()) return
        val x = (point.x - sheet.left).toDouble() / sheet.width() * 1000
        val y = (point.y - sheet.top).toDouble() / sheet.height() * 1400
        val margin = maxOf(3.0, (config?.strokeWidth ?: 3.0) * 2)
        addDamage(x - margin, y - margin, x + margin, y + margin)
    }

    /**
     * Clears a capacitive-panel cutout left behind by an older build. The app never arms one: it
     * kills every touch in that screen band, which is what made the soft keyboard unusable over
     * the sheet, and no reference implementation needs it — palm rejection comes from the limit
     * rect. See `/home/okhsunrog/tmp_zfs/reversed_onyx_notes_app/REPORT.md` (the stock app ships
     * `setAppCTPDisableRegion` with no callers) and
     * `/home/okhsunrog/tmp_zfs/reference_notes_apps/REPORT.md` (none of the five apps uses it).
     */
    private fun resetPalm() {
        EpdController.appResetCTPDisableRegion(activity)
    }

    fun close() {
        val hadSession = helper != null
        pause()
        generation++
        helper?.closeRawDrawing()
        // SurfaceFlinger holds the ink session and the exclusion table with no owner and no death
        // recipient, so whatever we leave behind outlives this process. Put both back by hand.
        if (hadSession) {
            cycleScribbleSession()
            clearLatchedExcludes()
        }
        helper = null
        config = null
        overlays = emptyList()
        swallowed = false
        region.reset()
        reconcile.reset()
        // Ink drawn under the partial mode leaves the sharpest ghosts; the sheet going away is
        // the moment to clean the panel once.
        if (hadSession) {
            webView.removeCallbacks(closeRefresh)
            webView.postDelayed(closeRefresh, CLOSE_REFRESH_DELAY_MS)
        }
    }

    private val closeRefresh = Runnable {
        runCatching { EpdController.invalidate(webView, UpdateMode.GC) }
            .onFailure { Log.d("OnyxInk", "refresh after close: ${it.message}") }
    }

    fun destroy() {
        close()
        webView.removeCallbacks(resumeGate)
        webView.viewTreeObserver.removeOnWindowFocusChangeListener(this)
        // The registry outlives this session; a keyboard held down here must not pause the next.
        pauses.resume(IME_REASON)
    }

    private fun send(kind: String, points: JSONArray? = null, erasing: Boolean = false) {
        val args = config ?: return
        emit(JSObject().apply {
            put("session", args.session)
            put("kind", kind)
            put("sequence", sequence)
            put("width", args.strokeWidth)
            put("erasing", erasing || args.eraser)
            put("fastPreview", fastPreview)
            if (points != null) put("points", points)
        })
    }

    private fun begin(point: TouchPoint, erasing: Boolean) {
        if (config == null || helper?.isRawDrawingInputEnabled != true) return
        if (!point.x.isFinite() || !point.y.isFinite() || !point.pressure.isFinite()) return
        // The firmware can skip an end callback; without this the palm region
        // and the transient display mode stay claimed until the next paired end.
        if (gesture.beginNeedsEnd()) end()
        val args = config ?: return
        val x = (point.x - sheet.left) / sheet.width() * 1000
        val y = (point.y - sheet.top) / sheet.height() * 1400
        val movingSelection = args.hasSelection && x >= args.selectionLeft - 12 && x <= args.selectionRight + 12 &&
            y >= args.selectionTop - 12 && y <= args.selectionBottom + 12
        // A pen landing on a floating control is a tap on that control, and the WebView delivers it
        // by itself. Swallow this one gesture rather than cutting the control out of the drawing
        // region: a hole in the region is also a hole in every trace crossing it, which is what
        // broke the dashed lasso outline.
        swallowed = inkOverlayHit(overlays, point.x, point.y)
        if (swallowed) {
            fastPreview = false
            setRender(false)
            gesture.begun()
            return
        }
        fastPreview = args.fastLasso && !erasing && !args.eraser && !movingSelection
        // Hardware erasing must pause the firmware pen layer even while Pen is selected.
        // Keep raw input enabled so the software eraser continues receiving points.
        eraserRenderGate.begin(erasing || args.eraser)
        if (args.interaction) setRender(fastPreview)
        gesture.begun()
        reconcile.began(erasing || args.eraser)
        webView.removeCallbacks(settleDisplay)
        // Only a gesture that genuinely animates asks for a transient panel mode: dragging a
        // selection around. Stock never puts the panel into one while handwriting or erasing.
        displayPolicy.begin(args.interaction && movingSelection)
        markNativePoint(point)
        webView.removeCallbacks(refresh)
        webView.removeCallbacks(penUpWait)
        cancelFrameSubmission()
        previewAt = 0L
        send("begin", JSONArray().put(normalize(point)), erasing)
    }

    private fun end() {
        if (!gesture.ended()) return
        if (swallowed) {
            swallowed = false
            restoreToolRendering()
            return
        }
        displayPolicy.end(SystemClock.uptimeMillis())
        if (displayPolicy.fastRequested) {
            webView.removeCallbacks(settleDisplay)
            webView.postDelayed(settleDisplay, InkRefreshPolicy.QUIET_MS)
        }
        send("end")
        // The submission in flight belongs to the frame this gesture superseded,
        // exactly as at pen-down.
        cancelFrameSubmission()
        // The reconcile itself is driven by the rendezvous in commit()/penUpRefresh(); this is
        // only the net for an acknowledgement that arrived mid-gesture and was never released.
        // It is timed so it can never preempt a pen-up refresh that is still on its way.
        webView.postDelayed(refresh, PEN_UP_WAIT_MS)
    }

    private fun stroke(list: TouchPointList, erasing: Boolean) {
        if (swallowed || !gesture.drawing || config == null || list.isEmpty) return
        val points = JSONArray()
        var pressure = 0.5
        // Same point budget as the portable draft. Never silently retain an unbounded native list.
        for (point in list.points.take(150_000)) {
            if (!point.x.isFinite() || !point.y.isFinite() || !point.pressure.isFinite()) continue
            if (point.pressure > 0) pressure = (point.pressure / maxPressure).toDouble().coerceIn(0.0, 1.0)
            markNativePoint(point)
            points.put(normalize(point, pressure))
        }
        if (points.length() > 0) {
            sequence++
            send("stroke", points, erasing)
        }
    }

    private fun normalize(point: TouchPoint, pressure: Double = (point.pressure / maxPressure).toDouble().coerceIn(0.0, 1.0)): JSONObject = JSONObject().apply {
        put("x", ((point.x - sheet.left) / sheet.width() * 1000).toDouble().coerceIn(0.0, 1000.0))
        put("y", ((point.y - sheet.top) / sheet.height() * 1400).toDouble().coerceIn(0.0, 1400.0))
        put("pressure", pressure)
        put("tiltX", point.tiltX.coerceIn(-90, 90))
        put("tiltY", point.tiltY.coerceIn(-90, 90))
        put("time", point.timestamp.coerceAtLeast(0))
    }

    private fun preview(point: TouchPoint, erasing: Boolean) {
        if (swallowed) return
        if (gesture.drawing) markNativePoint(point)
        if (!gesture.drawing || fastPreview || (config?.interaction != true && config?.eraser != true && !erasing)) return
        val now = android.os.SystemClock.uptimeMillis()
        if (now - previewAt < 32) return
        if (!point.x.isFinite() || !point.y.isFinite() || !point.pressure.isFinite()) return
        previewAt = now
        send("preview", JSONArray().put(normalize(point)), erasing)
    }

    private fun callback(epoch: Long) = object : RawInputCallback() {
        override fun onBeginRawDrawing(shortcut: Boolean, point: TouchPoint) { if (epoch == generation) begin(point, false) }
        override fun onEndRawDrawing(outside: Boolean, point: TouchPoint) { if (epoch == generation) end() }
        override fun onRawDrawingTouchPointMoveReceived(point: TouchPoint) { if (epoch == generation) preview(point, false) }
        override fun onRawDrawingTouchPointListReceived(points: TouchPointList) { if (epoch == generation) stroke(points, false) }
        override fun onBeginRawErasing(shortcut: Boolean, point: TouchPoint) { if (epoch == generation) begin(point, true) }
        override fun onEndRawErasing(outside: Boolean, point: TouchPoint) { if (epoch == generation) end() }
        override fun onRawErasingTouchPointMoveReceived(point: TouchPoint) { if (epoch == generation) preview(point, true) }
        override fun onRawErasingTouchPointListReceived(points: TouchPointList) { if (epoch == generation) stroke(points, true) }
        /** Delivered off the UI thread by an RxTimer; stock posts it to MAIN through EventBus. */
        override fun onPenUpRefresh(rect: RectF) {
            if (epoch != generation) return
            val copy = RectF(rect)
            webView.post { if (epoch == generation) penUpRefresh(copy) }
        }
    }
}
