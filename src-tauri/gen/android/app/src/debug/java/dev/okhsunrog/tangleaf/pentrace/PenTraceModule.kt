package dev.okhsunrog.tangleaf.pentrace

import android.graphics.Rect
import android.graphics.RectF
import android.os.Parcel
import android.util.Log
import android.view.View
import io.github.libxposed.api.XposedInterface
import io.github.libxposed.api.XposedModule
import io.github.libxposed.api.XposedModuleInterface.HotReloadedParam
import io.github.libxposed.api.XposedModuleInterface.HotReloadingParam
import io.github.libxposed.api.XposedModuleInterface.ModuleLoadedParam
import io.github.libxposed.api.XposedModuleInterface.PackageReadyParam
import io.github.libxposed.api.XposedModuleInterface.SystemServerStartingParam
import java.lang.reflect.Method
import java.lang.reflect.Modifier

/**
 * Debug-only research tracer: logs every call an app (stock Notes, Tangleaf) or system_server makes
 * into the Onyx pen and panel stack, so the sequence can be read next to the panel driver log
 * (`adb logcat -b all -v time | grep -E 'PenTrace|onyx_tcon|EUS|State change'`).
 *
 * It only observes: every hook proceeds with the original arguments and returns the original result.
 */
class PenTraceModule : XposedModule() {
    private var process = "?"
    private var appClassLoader: ClassLoader? = null
    private val hooked = HashSet<Method>()
    private val depth = ThreadLocal.withInitial { 0 }

    override fun onModuleLoaded(param: ModuleLoadedParam) {
        process = param.processName
    }

    override fun onSystemServerStarting(param: SystemServerStartingParam) {
        process = "system_server"
        install(null)
    }

    override fun onPackageReady(param: PackageReadyParam) {
        if (!param.isFirstPackage) return
        appClassLoader = param.classLoader
        install(param.classLoader)
    }

    override fun onHotReloading(param: HotReloadingParam): Boolean {
        param.setSavedInstanceState(arrayOf(process, appClassLoader))
        return true
    }

    override fun onHotReloaded(param: HotReloadedParam) {
        param.oldHookHandles.forEach { it.unhook() }
        val saved = param.savedInstanceState as? Array<*>
        process = saved?.getOrNull(0) as? String ?: param.processName
        appClassLoader = saved?.getOrNull(1) as? ClassLoader
        install(appClassLoader)
    }

    private fun install(app: ClassLoader?) {
        emit("installing in $process")
        // Framework side, present in every process: the single path to SurfaceFlinger, and the
        // Onyx additions to View that carry per-frame update modes. The transaction hook goes first
        // so that it, not the generic tracer, owns `transactData`.
        hookTransact()
        traceClass("android.onyx.ViewUpdateHelper", null) { isStatic(it) && !isQuery(it.name) }
        traceClass("android.onyx.optimization.EInkHelper", null) { isStatic(it) && !isQuery(it.name) }
        traceClass("android.view.View", null) { it.name in VIEW_METHODS && onyxViewOverload(it) }
        if (app == null) {
            // Firmware EinkWise policy acting on the foreground app.
            traceClass("android.onyx.optimization.impl.TabletEACRefreshImpl", null) { !isQuery(it.name) }
            traceClass("android.onyx.optimization.impl.EACBaseRefreshImpl", null) { !isQuery(it.name) }
            traceClass("android.onyx.optimization.impl.TabletEACDisplayImpl", null) { !isQuery(it.name) }
            traceClass("android.onyx.optimization.action.ValidateAppConfigAction", null) {
                it.name == "supportEAC"
            }
            traceClass("android.onyx.optimization.OECService", null) {
                OEC_METHODS.containsMatchIn(it.name) && !isQuery(it.name)
            }
        } else {
            // The Pen SDK bundled with the app: public intent, and the native reader entry points
            // (which reach SurfaceFlinger without going through ViewUpdateHelper).
            traceClass("com.onyx.android.sdk.pen.TouchHelper", app) {
                Modifier.isPublic(it.modifiers) && !isQuery(it.name) && it.name != "onTouchEvent"
            }
            traceClass("com.onyx.android.sdk.api.device.epd.EpdController", app) {
                isStatic(it) && Modifier.isPublic(it.modifiers) && !isQuery(it.name)
            }
            traceClass("com.onyx.android.sdk.pen.RawInputReader", app) {
                (Modifier.isPublic(it.modifiers) || Modifier.isNative(it.modifiers)) &&
                    !isQuery(it.name) && it.name != "onTouchPointReceived" && it.name != "nativeRawReader"
            }
            hookCallbackRegistration(app)
        }
    }

    /** Pen begin/end as the app sees them, for lining up with the driver log. */
    private fun hookCallbackRegistration(app: ClassLoader) {
        val reader = find("com.onyx.android.sdk.pen.RawInputReader", app) ?: return
        val setter = reader.declaredMethods.firstOrNull { it.name == "setRawInputCallback" } ?: return
        hook(setter).intercept { chain ->
            chain.getArg(0)?.let { traceCallback(it.javaClass) }
            chain.proceed()
        }
    }

    private fun traceCallback(type: Class<*>) {
        var cls: Class<*>? = type
        while (cls != null && cls != Any::class.java) {
            cls.declaredMethods
                .filter { it.name in CALLBACK_METHODS && !Modifier.isAbstract(it.modifiers) }
                .forEach { trace(it) { m, args -> "cb ${m.name}(${args.joinToString { fmt(it) }})" } }
            cls = cls.superclass
        }
    }

    private fun hookTransact() {
        val helper = find("android.onyx.ViewUpdateHelper", null) ?: return
        val codes =
            helper.declaredFields
                .filter { isStatic(it.modifiers) && it.type == Int::class.javaPrimitiveType }
                .mapNotNull { field ->
                    runCatching { field.isAccessible = true; field.getInt(null) to field.name }.getOrNull()
                }
                // Transaction codes live in two blocks, 0x10xxxx and 0xffxxxx; the update-mode
                // constants (0x80002, 0x200021, 0x500000, ...) fall outside both.
                .filter { (code, name) -> (code ushr 16) in TRANSACTION_BLOCKS && name == name.uppercase() }
                .toMap()
        val transact =
            helper.declaredMethods.firstOrNull {
                it.name == "transactData" &&
                    it.parameterTypes.contentEquals(
                        arrayOf(Int::class.javaPrimitiveType, Parcel::class.java, Parcel::class.java)
                    )
            } ?: return
        if (!hooked.add(transact)) return
        hook(transact).setExceptionMode(XposedInterface.ExceptionMode.PROTECTIVE).intercept { chain ->
            val code = chain.getArg(0) as Int
            val payload = (chain.getArg(1) as? Parcel)?.let(::payloadWords) ?: "?"
            emit("${indent()}SF ${codes[code] ?: "0x${code.toString(16)}"} $payload")
            chain.proceed()
        }
    }

    private fun traceClass(name: String, loader: ClassLoader?, filter: (Method) -> Boolean) {
        val cls = find(name, loader) ?: return
        cls.declaredMethods
            .filter { !Modifier.isAbstract(it.modifiers) && !it.isSynthetic && filter(it) }
            .forEach { method ->
                trace(method) { m, args ->
                    "${m.declaringClass.simpleName}.${m.name}(${args.joinToString { fmt(it) }})"
                }
            }
    }

    private fun trace(method: Method, describe: (Method, List<Any?>) -> String) {
        if (!hooked.add(method)) return
        runCatching {
            hook(method).setExceptionMode(XposedInterface.ExceptionMode.PROTECTIVE).intercept { chain ->
                val level = depth.get()!!
                val line = indent() + describe(method, chain.args)
                emit(line)
                if (method.name in STACK_METHODS) Log.i(TAG, "[$process] stack of ${method.name}", Throwable())
                depth.set(level + 1)
                try {
                    val result = chain.proceed()
                    if (method.returnType != Void.TYPE && method.returnType.isPrimitive)
                        emit("$line -> ${fmt(result)}")
                    result
                } finally {
                    depth.set(level)
                }
            }
        }.onFailure { emit("hook failed: ${method.declaringClass.name}.${method.name}: $it") }
    }

    private fun find(name: String, loader: ClassLoader?): Class<*>? =
        runCatching { Class.forName(name, false, loader ?: javaClass.classLoader) }
            .onFailure { emit("class missing: $name") }
            .getOrNull()

    private fun indent() = "  ".repeat(depth.get()!!)

    private fun emit(line: String) {
        Log.i(TAG, "[$process] ${Thread.currentThread().name.take(16)} $line")
    }

    companion object {
        private const val TAG = "PenTrace"
        private val VIEW_METHODS =
            setOf(
                "applyApplicationFastMode", "clearApplicationFastMode", "disableA2", "enableA2",
                "invalidate", "invalidateWithUpdateMode", "postInvalidate",
                "postInvalidateWithUpdateMode", "refreshScreen", "resetUpdateMode", "setDefaultUpdateMode",
            )
        private val CALLBACK_METHODS =
            setOf(
                "onBeginRawDrawing", "onEndRawDrawing", "onBeginRawErasing", "onEndRawErasing",
                "onPenActive", "onPenUpRefresh",
            )
        private val TRANSACTION_BLOCKS = setOf(0x10, 0xff)
        /** Calls whose origin is in question: log the Java stack that made them. */
        private val STACK_METHODS = setOf("beforeScroll")
        private val OEC_METHODS =Regex("(?i)resume|input|dither|debounc|refresh|scope|appconfig")

        private fun isStatic(m: Method) = Modifier.isStatic(m.modifiers)

        private fun isStatic(modifiers: Int) = Modifier.isStatic(modifiers)

        /** Getters and coordinate mapping run per frame or per point and change nothing. */
        private fun isQuery(name: String) =
            QUERY_PREFIXES.any { name.startsWith(it) } || name.startsWith("lambda$") || name in QUERY_NAMES

        private val QUERY_PREFIXES = listOf("get", "is", "has", "support", "map", "check", "can", "access$")
        private val QUERY_NAMES =
            setOf(
                "toString", "inSystemFastMode", "hashCode", "equals", "activityName", "noteViewDetect",
                "runOnUiThread", "createDummyEntryList", "surfaceComposerData", "dumpMessage",
            )

        /**
         * `View.invalidate()`/`postInvalidate()` without arguments are ordinary Android and fire
         * constantly; the Onyx overloads take an update mode (1 or 5 int arguments).
         */
        private fun onyxViewOverload(m: Method): Boolean {
            if (m.name != "invalidate" && m.name != "postInvalidate") return true
            val params = m.parameterTypes
            return (params.size == 1 || params.size == 5) &&
                params.all { it == Int::class.javaPrimitiveType }
        }

        /** The transaction payload after the interface token, as 32-bit words. */
        private fun payloadWords(parcel: Parcel): String {
            val saved = parcel.dataPosition()
            return try {
                parcel.setDataPosition(0)
                parcel.readInt() // strict mode policy
                parcel.readInt() // work source
                parcel.readInt() // 'SYST' header
                parcel.readString() // interface descriptor
                val words = ArrayList<String>()
                while (parcel.dataAvail() >= 4 && words.size < 64) words.add(word(parcel.readInt()))
                words.joinToString(prefix = "[", postfix = "]")
            } catch (e: Throwable) {
                "<${e.javaClass.simpleName}>"
            } finally {
                parcel.setDataPosition(saved)
            }
        }

        private fun word(value: Int): String {
            val asFloat = Float.fromBits(value)
            val floaty = value != 0 && (value > 0x10000000 || value < -0x10000000) && asFloat.isFinite() &&
                kotlin.math.abs(asFloat) in 1e-3f..1e6f
            return if (floaty) "$value(${asFloat}f)" else value.toString()
        }

        fun fmt(value: Any?): String =
            when (value) {
                null -> "null"
                is View -> "${value.javaClass.simpleName}#${Integer.toHexString(System.identityHashCode(value))}"
                is Rect -> value.toShortString()
                is RectF -> value.toShortString()
                is IntArray -> value.contentToString()
                is FloatArray -> value.contentToString()
                is BooleanArray -> value.contentToString()
                is Array<*> -> value.joinToString(prefix = "[", postfix = "]") { fmt(it) }
                is Collection<*> -> value.joinToString(prefix = "[", postfix = "]") { fmt(it) }
                is Enum<*> -> value.name
                is Number, is Boolean, is String -> value.toString()
                else -> value.toString().take(160)
            }
    }
}
