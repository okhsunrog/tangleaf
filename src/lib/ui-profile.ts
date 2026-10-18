import type { ProfilerOnRenderCallback } from "react";

// Diagnostic builds only. Entries contain timings, IDs and counts, never note text.
// Normal builds keep the regular React renderer and fold these branches away.
export const UI_PROFILE_ENABLED = import.meta.env.VITE_PROFILE_UI === "1";

type Detail = Record<string, string | number | boolean | null | undefined>;
type Entry = { name: string; start: number; end: number; detail?: Detail };
type ProfileWindow = Window & { __tangleafUiProfile?: (entry: Entry) => void };

function collector() {
  return UI_PROFILE_ENABLED ? (window as ProfileWindow).__tangleafUiProfile : undefined;
}

export function uiProfilePoint(name: string, detail?: Detail) {
  const collect = collector();
  if (!collect) return;
  const now = performance.now();
  collect({ name, start: now, end: now, detail });
}

export function uiProfileSpan<T>(name: string, operation: () => T, detail?: Detail): T {
  const collect = collector();
  if (!collect) return operation();
  const start = performance.now();
  try {
    return operation();
  } finally {
    collect({ name, start, end: performance.now(), detail });
  }
}

export const onUiProfileRender: ProfilerOnRenderCallback = (
  id,
  phase,
  actualDuration,
  baseDuration,
  startTime,
  commitTime,
) => {
  collector()?.({
    name: "react.render",
    start: startTime,
    end: commitTime,
    detail: { id, phase, actualDuration, baseDuration },
  });
};

/** Diagnostic method hooks, including version-specific CodeMirror internals.
 * Install even before a trace is armed so the first scheduled measurement is visible.
 * Each duration is inclusive; nested entries must not be added together.
 */
export function profileUiMethods(target: object, prefix: string, names: readonly string[]) {
  if (!UI_PROFILE_ENABLED) return () => {};
  const object = target as Record<string, unknown>;
  const restore: (() => void)[] = [];
  for (const name of names) {
    const original = object[name];
    if (typeof original !== "function") continue;
    const owned = Object.prototype.hasOwnProperty.call(object, name);
    const wrapper = function (this: unknown, ...args: unknown[]) {
      return uiProfileSpan(`${prefix}.${name}`, () => Reflect.apply(original, this, args));
    };
    object[name] = wrapper;
    restore.push(() => {
      if (object[name] !== wrapper) return;
      if (owned) object[name] = original;
      else delete object[name];
    });
  }
  return () => restore.forEach((undo) => undo());
}
