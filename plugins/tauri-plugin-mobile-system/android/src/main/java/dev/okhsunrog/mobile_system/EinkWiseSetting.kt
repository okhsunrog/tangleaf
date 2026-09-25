package dev.okhsunrog.mobile_system

/**
 * Whether EinkWise, the BOOX per-app optimisation service, handles this app.
 *
 * On, the firmware applies what the user set for us in EinkWise (DPI, contrast, refresh profile) and
 * also counts our taps into a full-screen refresh every `gcInterval` inputs and repaints after
 * input on its own. Off is how stock Notes runs: no counted refreshes and no post-input repaint, and
 * no EinkWise DPI either (docs/planning/eink-panel-upkeep.md §4). EinkWise stays on unless the user
 * turns it off from our settings; nothing writes the flag on its own.
 *
 * The flag is `supportEAC` in our app config, and the service keeps two copies of that config: the
 * per-package map a write lands in, and the app's theme, which is what it enforces and what
 * `getAppConfigFromService` answers from. The two are reconciled when the service starts, so a
 * change takes effect after the next reboot (seen on the Note Air 4C on 2026-09-26: the debouncer
 * stayed on after the write and was gone after the reboot). [State.enabled] reads the map, through
 * the service's list of apps with EinkWise off; [State.active] reads the theme. They differ while a
 * change waits for the reboot.
 */
internal class EinkWiseSetting(
    private val readStored: () -> Boolean?,
    private val readActive: () -> Boolean?,
    private val write: (Boolean) -> Unit,
) {
    data class State(val available: Boolean, val enabled: Boolean?, val active: Boolean?)

    fun state(): State {
        val stored = readStored()
        val active = readActive()
        return State(available = stored != null && active != null, enabled = stored, active = active)
    }

    /** Every write broadcasts a config change that comes back as a full redraw, so skip no-ops. */
    fun set(enabled: Boolean): State {
        val stored = readStored() ?: return state()
        if (stored != enabled) write(enabled)
        return state()
    }
}
