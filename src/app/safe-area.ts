/**
 * The native safe-area insets, copied into CSS variables.
 *
 * Android WebView does not populate `env(safe-area-inset-*)`, so the appearance provider writes
 * the values the plugin reports into `--safe-area-inset-*`. Two paths report them: a query on
 * window resize and rotation, and the plugin's `safeAreaChanged` event, which is the only one
 * that fires when a system bar is hidden or shown inside an edge-to-edge window (the window keeps
 * its size, so no resize event follows).
 *
 * Deliberately free of React: sheet mode waits for the next native change from an effect, and the
 * appearance provider is not something it can reach.
 */
export type SafeArea = { top: number; right: number; bottom: number; left: number };

const EDGES = ["top", "right", "bottom", "left"] as const;
type Edge = (typeof EDGES)[number];

/** The variables exactly as they were written, so they can be put back unchanged. */
export type SafeAreaVariables = Record<Edge, string>;

const variable = (edge: Edge) => `--safe-area-inset-${edge}`;

/** An edge the platform could not measure (the generated bindings allow null) counts as zero. */
export function writeSafeArea(
  area: Readonly<Record<Edge, number | null>>,
  root: HTMLElement = document.documentElement,
): void {
  for (const edge of EDGES) root.style.setProperty(variable(edge), `${area[edge] ?? 0}px`);
}

export function readSafeAreaVariables(
  root: HTMLElement = document.documentElement,
): SafeAreaVariables {
  const read = (edge: Edge) => root.style.getPropertyValue(variable(edge));
  return { top: read("top"), right: read("right"), bottom: read("bottom"), left: read("left") };
}

/**
 * Writes saved values back. An edge that had no value yet (the first native answer had not
 * arrived when it was saved) keeps its current one; the next native change corrects it.
 */
export function restoreSafeAreaVariables(
  saved: SafeAreaVariables,
  root: HTMLElement = document.documentElement,
): void {
  for (const edge of EDGES) {
    if (saved[edge]) root.style.setProperty(variable(edge), saved[edge]);
  }
}

const listeners = new Set<(area: SafeArea) => void>();

/** Calls `listener` for every native `safeAreaChanged` until the returned function is called. */
export function onNativeSafeAreaChange(listener: (area: SafeArea) => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The plugin's `safeAreaChanged` event: the variables follow first, then the listeners hear. */
export function nativeSafeAreaChanged(area: SafeArea): void {
  writeSafeArea(area);
  for (const listener of listeners) listener(area);
}
