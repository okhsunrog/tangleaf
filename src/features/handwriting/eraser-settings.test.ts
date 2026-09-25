// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import {
  DEFAULT_ERASER_SETTINGS,
  ERASER_TYPES,
  cssEraserRadius,
  diameterAt,
  diameterIndex,
  diameterPositions,
  eraserTitle,
  formatDiameter,
  normalizeDiameter,
  repairEraserSettings,
  resetEraserSettings,
  stepDiameter,
  useEraserSettings,
} from "./eraser-settings";

afterEach(() => {
  localStorage.clear();
  resetEraserSettings();
});

describe("eraser settings", () => {
  it("starts like a fresh stock install: stroke eraser, 0.50 mm stroke, 5.00 mm pixel", () => {
    expect(DEFAULT_ERASER_SETTINGS).toEqual({
      type: "stroke",
      diameters: { stroke: 50, pixel: 500, lasso: 50 },
      autoDeselect: false,
    });
    expect(ERASER_TYPES).toEqual(["pixel", "lasso", "stroke"]);
  });

  it("remembers a diameter per type", () => {
    const store = useEraserSettings.getState();
    store.setDiameter("pixel", 490);
    store.setType("stroke");
    store.setDiameter("stroke", 120);
    store.setType("pixel");
    expect(useEraserSettings.getState().diameters).toEqual({ stroke: 120, pixel: 490, lasso: 50 });
    expect(useEraserSettings.getState().type).toBe("pixel");
  });

  it("clamps the stepper at both ends", () => {
    expect(stepDiameter("pixel", 800, 1)).toBe(800);
    expect(stepDiameter("pixel", 100, -1)).toBe(100);
    expect(stepDiameter("stroke", 50, -1)).toBe(50);
    expect(stepDiameter("pixel", 500, -1)).toBe(495);
    expect(stepDiameter("stroke", 50, 1)).toBe(55);
    expect(diameterAt("pixel", -3)).toBe(100);
    expect(diameterAt("pixel", 500)).toBe(800);
  });

  it("maps diameters to slider positions and back", () => {
    expect(diameterPositions("pixel")).toBe(141);
    expect(diameterPositions("stroke")).toBe(151);
    expect(diameterIndex("pixel", 500)).toBe(80);
    expect(diameterAt("pixel", 80)).toBe(500);
    for (let index = 0; index < diameterPositions("stroke"); index++)
      expect(diameterIndex("stroke", diameterAt("stroke", index))).toBe(index);
  });

  it("titles each type like stock", () => {
    expect(eraserTitle("pixel", 500)).toBe("Pixel Eraser 5.00mm");
    expect(eraserTitle("stroke", 50)).toBe("Stroke Eraser 0.50mm");
    expect(eraserTitle("lasso", 50)).toBe("Lasso Eraser");
    expect(formatDiameter(495)).toBe("4.95mm");
    expect(formatDiameter(800)).toBe("8.00mm");
  });

  it("rounds to the 0.05 mm grid the way stock does, after clamping", () => {
    expect(normalizeDiameter("pixel", 498)).toBe(500);
    expect(normalizeDiameter("pixel", 497)).toBe(495);
    expect(normalizeDiameter("stroke", 20)).toBe(50);
    expect(normalizeDiameter("pixel", 900)).toBe(800);
    expect(normalizeDiameter("lasso", 500)).toBe(50);
    expect(normalizeDiameter("pixel", "5mm")).toBe(500);
    expect(normalizeDiameter("pixel", Number.NaN)).toBe(500);
  });

  it("repairs a stored state with an off-grid diameter and an unknown type", () => {
    expect(
      repairEraserSettings({
        type: "chalk",
        diameters: { stroke: 57, pixel: 1203 },
        autoDeselect: "yes",
      }),
    ).toEqual({
      type: "stroke",
      diameters: { stroke: 55, pixel: 800, lasso: 50 },
      autoDeselect: false,
    });
    expect(repairEraserSettings(null)).toEqual(DEFAULT_ERASER_SETTINGS);
  });

  it("repairs what storage hands back on rehydration", async () => {
    localStorage.setItem(
      "tangleaf.eraser-settings.v1",
      JSON.stringify({
        state: { type: "sponge", diameters: { stroke: 63, pixel: 1 }, autoDeselect: true },
        version: 1,
      }),
    );
    await useEraserSettings.persist.rehydrate();
    const state = useEraserSettings.getState();
    expect(state.type).toBe("stroke");
    expect(state.diameters).toEqual({ stroke: 65, pixel: 100, lasso: 50 });
    expect(state.autoDeselect).toBe(true);
  });

  it("persists every change at once", () => {
    useEraserSettings.getState().setAutoDeselect(true);
    useEraserSettings.getState().setDiameter("pixel", 505);
    const stored = JSON.parse(localStorage.getItem("tangleaf.eraser-settings.v1")!);
    expect(stored.state).toEqual({
      type: "stroke",
      diameters: { stroke: 50, pixel: 505, lasso: 50 },
      autoDeselect: true,
    });
  });

  it("measures an eraser in CSS millimetres where the plugin gives no radius", () => {
    // 5.00 mm across is 9.45 CSS px of radius; a 500 px wide sheet has 2 units per CSS px.
    expect(cssEraserRadius(500, 500)).toBeCloseTo(18.9, 1);
    expect(cssEraserRadius(500, 0)).toBe(0);
  });
});
