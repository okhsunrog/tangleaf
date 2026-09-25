import { useEffect, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X } from "lucide-react";
import { loadSettings, type WindowButton, type WindowControlsStyle } from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { cn } from "@/lib/utils";
import {
  FALLBACK_SYSTEM_CONTROLS,
  resolveWindowControls,
  type ResolvedWindowControlsStyle,
} from "./window-controls-layout";

const LABELS: Record<WindowButton, string> = {
  minimize: "Minimize window",
  maximize: "Maximize or restore window",
  close: "Close window",
};

interface WindowControlsProps {
  /** Which end of the title bar this instance sits at; it shows only that side's buttons. */
  side: "left" | "right";
  /** Overrides the saved style, for previewing a choice before it is saved. */
  style?: WindowControlsStyle;
  /** Draws the buttons without acting on the window. */
  preview?: boolean;
}

/**
 * Title bar buttons for borderless mode, drawn to look like the desktop's own. Every colour comes
 * from the `--window-control-*` variables, which default to the active palette (see index.css).
 */
export function WindowControls({ side, style, preview = false }: WindowControlsProps) {
  const settings = useQuery({ queryKey: queryKeys.settings, queryFn: loadSettings });
  const layout = resolveWindowControls(
    style ?? settings.data?.windowControlsStyle ?? "auto",
    settings.data?.systemWindowControls ?? FALLBACK_SYSTEM_CONTROLS,
  );
  const maximized = useMaximized(!preview);
  const buttons = side === "left" ? layout.left : layout.right;
  if (buttons.length === 0) return null;

  const act = (button: WindowButton) => {
    if (preview) return;
    const window = getCurrentWindow();
    if (button === "minimize") void window.minimize();
    else if (button === "maximize") void window.toggleMaximize();
    else void window.close();
  };

  return (
    <div
      className={cn("window-controls flex shrink-0 items-center", GROUP_CLASS[layout.style])}
      data-window-controls={layout.style}
      aria-label={preview ? undefined : "Window controls"}
      aria-hidden={preview ? true : undefined}
    >
      {buttons.map((button) => (
        <button
          key={button}
          type="button"
          tabIndex={preview ? -1 : undefined}
          aria-label={LABELS[button]}
          className={cn("group/control", BUTTON_CLASS[layout.style](button))}
          onClick={() => act(button)}
        >
          {renderGlyph(layout.style, button, maximized)}
        </button>
      ))}
    </div>
  );
}

function useMaximized(track: boolean) {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    if (!track) return;
    const appWindow = getCurrentWindow();
    let disposed = false;
    const refresh = () => {
      void appWindow
        .isMaximized()
        .then((value) => {
          if (!disposed) setMaximized(value);
        })
        .catch(console.error);
    };
    const listener = appWindow.onResized(refresh);
    refresh();
    return () => {
      disposed = true;
      void listener.then((unlisten) => unlisten()).catch(console.error);
    };
  }, [track]);
  return maximized;
}

const GROUP_CLASS: Record<ResolvedWindowControlsStyle, string> = {
  breeze: "gap-1.5",
  adwaita: "gap-3",
  windows: "self-stretch",
  macos: "gap-2 px-1",
  minimal: "",
};

// Breeze: a bare glyph that gets a filled circle on hover; close hovers in the palette's
// destructive colour. Measured against KWin's Breeze: 18 px circle, 24 px pitch.
const breezeButton = (button: WindowButton) =>
  cn(
    "flex size-[18px] items-center justify-center rounded-full text-(--window-control-fg) outline-none transition-colors",
    "focus-visible:ring-2 focus-visible:ring-ring/60",
    button === "close"
      ? "hover:bg-(--window-control-close) hover:text-(--window-control-close-fg) active:bg-(--window-control-close)/80"
      : "hover:bg-(--window-control-hover) hover:text-(--window-control-hover-fg) active:bg-(--window-control-hover)/80",
  );

// Adwaita: always-visible translucent circles, darker on hover and press.
const adwaitaButton = () =>
  cn(
    "flex size-6 items-center justify-center rounded-full text-(--window-control-fg) outline-none transition-colors",
    "bg-(--window-control-fg)/10 hover:bg-(--window-control-fg)/15 active:bg-(--window-control-fg)/30",
    "focus-visible:ring-2 focus-visible:ring-ring/60",
  );

// Windows 11: wide square-cornered cells; close turns the destructive colour.
const windowsButton = (button: WindowButton) =>
  cn(
    "flex h-full min-h-8 w-[46px] items-center justify-center text-(--window-control-fg) outline-none transition-colors",
    "focus-visible:bg-(--window-control-fg)/10",
    button === "close"
      ? "hover:bg-(--window-control-close) hover:text-(--window-control-close-fg)"
      : "hover:bg-(--window-control-fg)/10 active:bg-(--window-control-fg)/15",
  );

// macOS: the traffic lights keep their own colours; glyphs show while the group is hovered.
const MACOS_FILL: Record<WindowButton, string> = {
  close: "bg-[#ff5f57] border-[#e0443e]",
  minimize: "bg-[#febc2e] border-[#dea123]",
  maximize: "bg-[#28c840] border-[#1aab29]",
};
const macosButton = (button: WindowButton) =>
  cn(
    "flex size-3 items-center justify-center rounded-full border-[0.5px] text-black/60 outline-none",
    "focus-visible:ring-2 focus-visible:ring-ring/60",
    MACOS_FILL[button],
  );

const minimalButton = (button: WindowButton) =>
  cn(
    "flex size-8 items-center justify-center rounded-sm outline-none",
    button === "close"
      ? "hover:bg-destructive hover:text-destructive-foreground"
      : "hover:bg-accent",
  );

const BUTTON_CLASS: Record<ResolvedWindowControlsStyle, (button: WindowButton) => string> = {
  breeze: breezeButton,
  adwaita: adwaitaButton,
  windows: windowsButton,
  macos: macosButton,
  minimal: minimalButton,
};

function renderGlyph(
  style: ResolvedWindowControlsStyle,
  button: WindowButton,
  maximized: boolean,
): ReactNode {
  switch (style) {
    case "breeze":
      return <BreezeGlyph button={button} maximized={maximized} />;
    case "adwaita":
      return <AdwaitaGlyph button={button} maximized={maximized} />;
    case "windows":
      return <WindowsGlyph button={button} maximized={maximized} />;
    case "macos":
      return <MacosGlyph button={button} />;
    case "minimal":
      return button === "minimize" ? (
        <Minus className="size-4" />
      ) : button === "maximize" ? (
        <Square className="size-3" />
      ) : (
        <X className="size-4" />
      );
  }
}

interface GlyphProps {
  button: WindowButton;
  maximized: boolean;
}

/** Breeze's own paths, on its 18-unit button canvas. */
function BreezeGlyph({ button, maximized }: GlyphProps) {
  return (
    <svg
      viewBox="0 0 18 18"
      className="size-[18px]"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.1}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      {button === "minimize" && <polyline points="4,7 9,12 14,7" />}
      {button === "maximize" &&
        (maximized ? <polygon points="4,9 9,4 14,9 9,14" /> : <polyline points="4,11 9,6 14,11" />)}
      {button === "close" && <path d="M5 5 L13 13 M13 5 L5 13" />}
    </svg>
  );
}

/** GNOME's symbolic window icons, 16 units. */
function AdwaitaGlyph({ button, maximized }: GlyphProps) {
  return (
    <svg
      viewBox="0 0 16 16"
      className="size-4"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      {button === "minimize" && <path d="M4.5 11.25 H11.5" />}
      {button === "maximize" &&
        (maximized ? (
          <>
            <rect x="3.75" y="5.75" width="6.5" height="6.5" rx="1" />
            <path d="M6 3.75 H11.25 A1 1 0 0 1 12.25 4.75 V10" />
          </>
        ) : (
          <rect x="4" y="4" width="8" height="8" rx="1.25" />
        ))}
      {button === "close" && <path d="M4.5 4.5 L11.5 11.5 M11.5 4.5 L4.5 11.5" />}
    </svg>
  );
}

/** Segoe Fluent's caption glyphs, drawn at 10 units with hairlines. */
function WindowsGlyph({ button, maximized }: GlyphProps) {
  return (
    <svg
      viewBox="0 0 10 10"
      className="size-2.5"
      fill="none"
      stroke="currentColor"
      strokeWidth={1}
      shapeRendering="crispEdges"
      aria-hidden
    >
      {button === "minimize" && <path d="M0 5 H10" />}
      {button === "maximize" &&
        (maximized ? (
          <>
            <rect x="0.5" y="2.5" width="7" height="7" />
            <path d="M2.5 2.5 V0.5 H9.5 V7.5 H7.5" />
          </>
        ) : (
          <rect x="0.5" y="0.5" width="9" height="9" />
        ))}
      {button === "close" && (
        <path d="M0 0 L10 10 M10 0 L0 10" shapeRendering="geometricPrecision" />
      )}
    </svg>
  );
}

function MacosGlyph({ button }: { button: WindowButton }) {
  return (
    <svg
      viewBox="0 0 12 12"
      className="size-3 opacity-0 transition-opacity [.window-controls:hover_&]:opacity-100"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.2}
      strokeLinecap="round"
      aria-hidden
    >
      {button === "close" && <path d="M4 4 L8 8 M8 4 L4 8" />}
      {button === "minimize" && <path d="M3.5 6 H8.5" />}
      {button === "maximize" && <path d="M6 3.5 V8.5 M3.5 6 H8.5" />}
    </svg>
  );
}
