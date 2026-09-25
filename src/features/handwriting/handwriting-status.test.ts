import { describe, expect, it } from "vite-plus/test";
import { byPriority, statusLabel, type NoteStatus } from "./handwriting-status";
import { toolbarLayout } from "./handwriting-toolbar";

const noop = () => {};
const saveError: NoteStatus = { kind: "save-error", message: "disk full", onRetry: noop };
const titleConflict: NoteStatus = {
  kind: "title-conflict",
  onUseRemote: noop,
  onKeepLocal: noop,
};
const versions: NoteStatus = {
  kind: "versions",
  count: 3,
  compareDisabled: false,
  onCompare: noop,
};
const sheetFull: NoteStatus = { kind: "sheet-full" };
const penError: NoteStatus = { kind: "pen-error", message: "no firmware" };

describe("note status", () => {
  it("puts a failing save first and a pen fallback last", () => {
    expect(byPriority([penError, sheetFull, versions, titleConflict, saveError])).toEqual([
      saveError,
      titleConflict,
      versions,
      sheetFull,
      penError,
    ]);
    expect(byPriority([])).toEqual([]);
  });

  it("labels the chip in a word or two", () => {
    expect([saveError, titleConflict, versions, sheetFull, penError].map(statusLabel)).toEqual([
      "Not saved",
      "Title conflict",
      "3 versions",
      "Sheet full",
      "Pen input",
    ]);
  });
});

describe("toolbarLayout", () => {
  it("keeps the pen presets from 600 CSS px, like the tablet in either orientation", () => {
    expect(toolbarLayout(661)).toEqual({ penPresets: true, statusIconOnly: false });
    expect(toolbarLayout(600)).toEqual({ penPresets: true, statusIconOnly: false });
    expect(toolbarLayout(599)).toEqual({ penPresets: false, statusIconOnly: false });
  });

  it("shows only the status icon on a phone", () => {
    expect(toolbarLayout(479)).toEqual({ penPresets: false, statusIconOnly: true });
    expect(toolbarLayout(360)).toEqual({ penPresets: false, statusIconOnly: true });
  });

  it("uses the wide bar until the bar has been measured", () => {
    expect(toolbarLayout(0)).toEqual({ penPresets: true, statusIconOnly: false });
  });
});
