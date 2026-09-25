import type { SystemWindowControls, WindowButton, WindowControlsStyle } from "@/lib/api";

export type ResolvedWindowControlsStyle = Exclude<WindowControlsStyle, "auto">;

export interface WindowControlsLayout {
  style: ResolvedWindowControlsStyle;
  left: WindowButton[];
  right: WindowButton[];
}

const ALL: WindowButton[] = ["minimize", "maximize", "close"];

export const WINDOW_CONTROLS_STYLES: WindowControlsStyle[] = [
  "auto",
  "breeze",
  "adwaita",
  "windows",
  "macos",
  "minimal",
];

export const WINDOW_CONTROLS_STYLE_NAMES: Record<WindowControlsStyle, string> = {
  auto: "Match the desktop",
  breeze: "KDE Breeze",
  adwaita: "GNOME Adwaita",
  windows: "Windows 11",
  macos: "macOS",
  minimal: "Minimal",
};

/** Where each look puts its buttons when it is not the desktop's own. */
const CONVENTIONAL: Record<ResolvedWindowControlsStyle, Omit<WindowControlsLayout, "style">> = {
  breeze: { left: [], right: ALL },
  adwaita: { left: [], right: ALL },
  windows: { left: [], right: ALL },
  macos: { left: ["close", "minimize", "maximize"], right: [] },
  minimal: { left: [], right: ALL },
};

/** Used before settings load (or when they cannot), so the window stays closable. */
export const FALLBACK_SYSTEM_CONTROLS: SystemWindowControls = {
  style: "minimal",
  left: [],
  right: ALL,
};

/**
 * `auto`, or the look the desktop itself uses, follows the desktop's button sides and order; any
 * other look brings its own conventional layout, since the desktop's may not suit it.
 */
export function resolveWindowControls(
  style: WindowControlsStyle,
  system: SystemWindowControls,
): WindowControlsLayout {
  if (style === "auto" || style === system.style) {
    const resolved = system.style === "auto" ? "minimal" : system.style;
    return { style: resolved, left: system.left, right: system.right };
  }
  return { style, ...CONVENTIONAL[style] };
}
