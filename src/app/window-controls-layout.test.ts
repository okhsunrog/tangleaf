import { describe, expect, it } from "vite-plus/test";
import type { SystemWindowControls } from "@/lib/api";
import { resolveWindowControls } from "./window-controls-layout";

const kdeCloseOnLeft: SystemWindowControls = {
  style: "breeze",
  left: ["close"],
  right: ["minimize", "maximize"],
};

describe("window controls layout", () => {
  it("auto follows the desktop's look and button sides", () => {
    expect(resolveWindowControls("auto", kdeCloseOnLeft)).toEqual({
      style: "breeze",
      left: ["close"],
      right: ["minimize", "maximize"],
    });
  });

  it("choosing the desktop's own look keeps the desktop's layout", () => {
    expect(resolveWindowControls("breeze", kdeCloseOnLeft).left).toEqual(["close"]);
  });

  it("another look brings its conventional layout", () => {
    expect(resolveWindowControls("macos", kdeCloseOnLeft)).toEqual({
      style: "macos",
      left: ["close", "minimize", "maximize"],
      right: [],
    });
    expect(resolveWindowControls("windows", kdeCloseOnLeft)).toEqual({
      style: "windows",
      left: [],
      right: ["minimize", "maximize", "close"],
    });
  });
});
