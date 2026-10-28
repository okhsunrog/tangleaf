export const NAVIGATION_HIGHLIGHT_MS = 1_800;

const timers = new WeakMap<HTMLElement, ReturnType<typeof setTimeout>>();

/** A static highlight also works on e-ink and with reduced motion. */
export function highlightNavigationTarget(element: HTMLElement) {
  clearTimeout(timers.get(element));
  element.classList.add("navigation-target-highlight");
  timers.set(
    element,
    setTimeout(() => {
      element.classList.remove("navigation-target-highlight");
      timers.delete(element);
    }, NAVIGATION_HIGHLIGHT_MS),
  );
}
