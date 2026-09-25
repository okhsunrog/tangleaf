import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { EraserMode } from "./ink-editing";
import { SHEET_BOUNDS } from "./ink-region";

/**
 * The eraser's settings, in the stock Notes app's own units.
 *
 * A diameter is an integer in hundredths of a millimetre, the unit stock stores and steps in
 * (`NoteUnits`): 500 is 5.00 mm. Stock converts it to device pixels only when it hands the width to
 * the firmware, and so does the plugin here; the page never measures an eraser in sheet units.
 *
 * The settings are global on this device. Stock keeps the type globally and the widths per
 * notebook; one set for every note is the simpler rule, and the tool itself is never remembered —
 * a note always opens with the pen.
 */

/** Left to right in the menu, as in stock (Pixel, Lasso, Stroke). */
export const ERASER_TYPES = ["pixel", "lasso", "stroke"] as const satisfies readonly EraserMode[];

export const ERASER_NAMES: Record<EraserMode, string> = {
  pixel: "Pixel Eraser",
  lasso: "Lasso Eraser",
  stroke: "Stroke Eraser",
};

/** One press of `<` or `>`, and one slider position: 0.05 mm. */
export const DIAMETER_STEP = 5;

/**
 * Stock's ranges and defaults. The lasso eraser has no diameter of its own; stock still keeps
 * 0.50 mm for it and hands that to the firmware, and never shows it.
 */
const DIAMETERS: Record<EraserMode, { min: number; max: number; initial: number }> = {
  stroke: { min: 50, max: 800, initial: 50 },
  pixel: { min: 100, max: 800, initial: 500 },
  lasso: { min: 50, max: 50, initial: 50 },
};

/** Stock's type for a fresh install. */
export const DEFAULT_ERASER_TYPE: EraserMode = "stroke";

export function diameterRange(type: EraserMode) {
  return DIAMETERS[type];
}

/** How many values the slider offers: 151 for the stroke eraser, 141 for the pixel eraser. */
export function diameterPositions(type: EraserMode): number {
  const { min, max } = DIAMETERS[type];
  return (max - min) / DIAMETER_STEP + 1;
}

/**
 * Clamp to the type's range, then round to the 0.05 mm grid the way stock does
 * (`NumberUtils.roundToNearestMultipleOfFive`: a remainder below 3 rounds down, anything else up).
 * Anything that is not a number falls back to the type's default.
 */
export function normalizeDiameter(type: EraserMode, diameter: unknown): number {
  const { min, max, initial } = DIAMETERS[type];
  if (typeof diameter !== "number" || !Number.isFinite(diameter)) return initial;
  const clamped = Math.min(max, Math.max(min, Math.trunc(diameter)));
  const remainder = clamped % DIAMETER_STEP;
  return remainder < 3 ? clamped - remainder : clamped + (DIAMETER_STEP - remainder);
}

/** The slider position of a diameter. */
export function diameterIndex(type: EraserMode, diameter: number): number {
  return (normalizeDiameter(type, diameter) - DIAMETERS[type].min) / DIAMETER_STEP;
}

/** The diameter at a slider position, clamped to the ends. */
export function diameterAt(type: EraserMode, index: number): number {
  const last = diameterPositions(type) - 1;
  const clamped = Math.min(last, Math.max(0, Math.round(index)));
  return DIAMETERS[type].min + clamped * DIAMETER_STEP;
}

/** `<` and `>`: one step, and nothing past either end. */
export function stepDiameter(type: EraserMode, diameter: number, direction: 1 | -1): number {
  return diameterAt(type, diameterIndex(type, diameter) + direction);
}

/** `5.00mm`, with no space before the unit, as stock prints it. */
export function formatDiameter(diameter: number): string {
  const whole = Math.floor(diameter / 100);
  const hundredths = String(diameter % 100).padStart(2, "0");
  return `${whole}.${hundredths}mm`;
}

/** `Pixel Eraser 5.00mm`, `Stroke Eraser 0.50mm`, or just `Lasso Eraser`. */
export function eraserTitle(type: EraserMode, diameter: number): string {
  const name = ERASER_NAMES[type];
  return type === "lasso" ? name : `${name} ${formatDiameter(diameter)}`;
}

/**
 * The eraser's reach in sheet units where no native value is available (the pointer path on
 * desktop, or a plugin that reported none): the diameter in CSS millimetres over a sheet
 * `sheetCssWidth` CSS px wide. On the BOOX WebView CSS millimetres are not physical ones, which is
 * why the pen session measures the eraser natively there.
 */
export function cssEraserRadius(diameter: number, sheetCssWidth: number): number {
  if (!(sheetCssWidth > 0)) return 0;
  const radiusCss = (diameter / 200) * (96 / 25.4);
  return (radiusCss * SHEET_BOUNDS.right) / sheetCssWidth;
}

export type EraserSettings = {
  type: EraserMode;
  diameters: Record<EraserMode, number>;
  autoDeselect: boolean;
};

export const DEFAULT_ERASER_SETTINGS: EraserSettings = {
  type: DEFAULT_ERASER_TYPE,
  diameters: {
    stroke: DIAMETERS.stroke.initial,
    pixel: DIAMETERS.pixel.initial,
    lasso: DIAMETERS.lasso.initial,
  },
  autoDeselect: false,
};

function isEraserType(value: unknown): value is EraserMode {
  return typeof value === "string" && (ERASER_TYPES as readonly string[]).includes(value);
}

/**
 * Whatever storage returns is repaired before the menu sees it: `persist` validates nothing, and an
 * older build or a hand-edited value must not put the slider off its grid or out of range.
 */
export function repairEraserSettings(stored: unknown): EraserSettings {
  const value = (typeof stored === "object" && stored !== null ? stored : {}) as Partial<
    Record<keyof EraserSettings, unknown>
  >;
  const diameters = (
    typeof value.diameters === "object" && value.diameters !== null ? value.diameters : {}
  ) as Partial<Record<EraserMode, unknown>>;
  return {
    type: isEraserType(value.type) ? value.type : DEFAULT_ERASER_TYPE,
    diameters: {
      stroke: normalizeDiameter("stroke", diameters.stroke),
      pixel: normalizeDiameter("pixel", diameters.pixel),
      lasso: normalizeDiameter("lasso", diameters.lasso),
    },
    autoDeselect: typeof value.autoDeselect === "boolean" ? value.autoDeselect : false,
  };
}

type EraserSettingsStore = EraserSettings & {
  /** Selects a type; its own remembered diameter comes back with it. */
  setType: (type: EraserMode) => void;
  setDiameter: (type: EraserMode, diameter: number) => void;
  setAutoDeselect: (autoDeselect: boolean) => void;
};

export const useEraserSettings = create<EraserSettingsStore>()(
  persist(
    (set) => ({
      ...DEFAULT_ERASER_SETTINGS,
      setType: (type) => set({ type }),
      setDiameter: (type, diameter) =>
        set((state) => ({
          diameters: { ...state.diameters, [type]: normalizeDiameter(type, diameter) },
        })),
      setAutoDeselect: (autoDeselect) => set({ autoDeselect }),
    }),
    {
      name: "tangleaf.eraser-settings.v1",
      version: 1,
      partialize: ({ type, diameters, autoDeselect }) => ({ type, diameters, autoDeselect }),
      // Nothing older exists; whatever was stored is repaired by `merge` either way.
      migrate: (stored) => stored as EraserSettingsStore,
      merge: (stored, current) => ({ ...current, ...repairEraserSettings(stored) }),
    },
  ),
);

/** Test seam: back to a fresh install. */
export function resetEraserSettings(): void {
  useEraserSettings.setState(DEFAULT_ERASER_SETTINGS);
}
