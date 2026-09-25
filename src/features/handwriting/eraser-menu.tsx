import { useState, type CSSProperties } from "react";
import { Slider } from "@base-ui/react/slider";
import { Info } from "lucide-react";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import { ErasePageIcon, EraserTypeIcon } from "./eraser-icons";
import {
  ERASER_NAMES,
  ERASER_TYPES,
  diameterAt,
  diameterIndex,
  diameterPositions,
  eraserTitle,
  formatDiameter,
  stepDiameter,
  useEraserSettings,
} from "./eraser-settings";
import type { EraserMode } from "./ink-editing";

/**
 * The eraser menu, laid out like stock Notes' `ErasePopup` on the Note Air 4C.
 *
 * Stock's sizes are its own dp at density 1.875. This WebView runs at a higher, per-app density, so
 * a stock dp is not a CSS px here: every length is `n × var(--dp)`, with `--dp` = 1.875 / DPR when
 * the BOOX pen is present (2/3 CSS px on the tablet, which makes the menu as wide on the glass as
 * stock's), and 1 CSS px elsewhere. Lines stock draws in device pixels use `--dpx` = one device
 * pixel. Nothing here is in rem: rem follows the system font scale, and stock's dp do not.
 */

/** Stock Notes' density on this panel (300 dpi / 160). */
const STOCK_DENSITY = 1.875;
/** Below a 40 px button in the 44 px toolbar lie 1.5 px of bar and its 1 px rule. */
const BUTTON_TO_TOOLBAR_RULE_PX = 2.5;
/** Stock hangs the menu 23 device px (12.3 dp) below its toolbar's rule. */
const MENU_BELOW_RULE_DP = 12.3;
/** Stock's hint sits `max(divider, 4 px)` above the ⓘ. */
const HINT_GAP_DEVICE_PX = 4;

/** A 3-point wedge: 1 dp at its left end, as tall as its box at the right. */
const WEDGE_CLIP =
  "polygon(0 calc(50% - 0.5 * var(--dp)), 100% 0, 100% 100%, 0 calc(50% + 0.5 * var(--dp)))";

export type EraserMenuUnits = { dp: number; dpx: number };

export function eraserMenuUnits(nativePen: boolean): EraserMenuUnits {
  const ratio = window.devicePixelRatio || 1;
  return { dp: nativePen ? STOCK_DENSITY / ratio : 1, dpx: 1 / ratio };
}

/** The two units, set on every popup the menu shows; a portal does not inherit them. */
export function eraserMenuStyle(units: EraserMenuUnits): CSSProperties {
  return { "--dp": `${units.dp}px`, "--dpx": `${units.dpx}px` } as CSSProperties;
}

export function eraserMenuSideOffset(units: EraserMenuUnits): number {
  return BUTTON_TO_TOOLBAR_RULE_PX + MENU_BELOW_RULE_DP * units.dp;
}

/**
 * The popup itself: stock's 483 dp, white, a 3 px black border with a 19 px radius and no shadow.
 * On e-ink the display profile's own 1 px popover border wins, which is close enough.
 */
export const ERASER_MENU_CLASS =
  "w-[calc(483*var(--dp))] max-w-[calc(100vw-24px)] rounded-[calc(19*var(--dpx))] border-[length:calc(3*var(--dpx))] border-foreground p-0 shadow-none";

type Props = {
  units: EraserMenuUnits;
  /** Erase Page needs the draft; a stroke, an undo or a leave in flight holds it. */
  eraseDisabled: boolean;
  /** A type was chosen: stock closes the menu on every type tap, the selected type's too. */
  onTypeChosen: () => void;
  onErasePage: () => void;
};

export function EraserMenu({ units, eraseDisabled, onTypeChosen, onErasePage }: Props) {
  const type = useEraserSettings((state) => state.type);
  const stored = useEraserSettings((state) => state.diameters[state.type]);
  const autoDeselect = useEraserSettings((state) => state.autoDeselect);
  const setType = useEraserSettings((state) => state.setType);
  const setDiameter = useEraserSettings((state) => state.setDiameter);
  const setAutoDeselect = useEraserSettings((state) => state.setAutoDeselect);
  // The value under a dragging thumb. The title and the value follow it; the store takes it on
  // release, and the eraser itself only once the menu closes.
  const [live, setLive] = useState<number | null>(null);
  const shown = live ?? stored;

  const step = (direction: 1 | -1) => {
    setLive(null);
    setDiameter(type, stepDiameter(type, shown, direction));
  };

  return (
    <div
      data-eraser-menu
      className="pt-[calc(28.6*var(--dp))] text-[length:calc(20*var(--dp))] leading-[1.17]"
    >
      <p className="px-[calc(21.2*var(--dp))]">{eraserTitle(type, shown)}</p>
      {/* Radios, not pressed buttons: a pressed button is painted solid black on e-ink, and
          stock marks the choice with a dot under the icon instead. */}
      <div
        role="radiogroup"
        aria-label="Eraser type"
        className="mt-[calc(13.8*var(--dp))] flex px-[calc(92.2*var(--dp))]"
      >
        {ERASER_TYPES.map((value: EraserMode) => (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={value === type}
            aria-label={ERASER_NAMES[value]}
            className="relative flex h-[calc(91.1*var(--dp))] min-w-0 flex-1 items-center justify-center"
            onClick={() => {
              setType(value);
              onTypeChosen();
            }}
          >
            <EraserTypeIcon type={value} className="size-[calc(33.9*var(--dp))]" />
            {value === type && (
              <span
                aria-hidden
                className="absolute bottom-[calc(11.7*var(--dp))] left-1/2 size-[calc(11*var(--dp))] -translate-x-1/2 rounded-full bg-current"
              />
            )}
          </button>
        ))}
      </div>
      <Rule />
      {type !== "lasso" && (
        <>
          <div className="mt-[calc(21.2*var(--dp))] flex h-[calc(23.4*var(--dp))] items-center px-[calc(21.2*var(--dp))]">
            <span className="min-w-0 flex-1 truncate">Eraser Diameter</span>
            <StepButton label="Decrease eraser diameter" direction={-1} onClick={() => step(-1)} />
            <span className="mx-[calc(21.2*var(--dp))] shrink-0 tabular-nums">
              {formatDiameter(shown)}
            </span>
            <StepButton label="Increase eraser diameter" direction={1} onClick={() => step(1)} />
          </div>
          <Slider.Root
            value={diameterIndex(type, shown)}
            min={0}
            max={diameterPositions(type) - 1}
            step={1}
            onValueChange={(index) => setLive(diameterAt(type, index))}
            onValueCommitted={(index) => {
              setLive(null);
              setDiameter(type, diameterAt(type, index));
            }}
            className="mt-[calc(21.2*var(--dp))] px-[calc(21.2*var(--dp))]"
          >
            {/* The thumb's centre runs between the ends of the wedge, so the control is inset
                by half a thumb and the thumb overhangs it, as stock's seek bar does. */}
            <Slider.Control className="flex h-[calc(34*var(--dp))] touch-none items-center px-[calc(17*var(--dp))]">
              <Slider.Track className="h-full w-full">
                <span
                  aria-hidden
                  className="absolute top-1/2 right-[calc(13.5*var(--dp))] left-0 h-[calc(27*var(--dp))] -translate-y-1/2 bg-current"
                  style={{ clipPath: WEDGE_CLIP }}
                />
                <span
                  aria-hidden
                  className="absolute top-1/2 right-0 size-[calc(27*var(--dp))] -translate-y-1/2 rounded-full bg-current"
                />
                <Slider.Thumb
                  aria-label="Eraser diameter"
                  getAriaValueText={() => formatDiameter(shown)}
                  className="size-[calc(34*var(--dp))] rounded-full border-[length:calc(2.83*var(--dp))] border-current bg-popover"
                />
              </Slider.Track>
            </Slider.Control>
          </Slider.Root>
          <Rule inset className="mt-[calc(21.2*var(--dp))]" />
        </>
      )}
      {/* "Do Not Erase Shapes" is left out: there are no shapes to spare. */}
      <div className="box-content flex h-[calc(23.4*var(--dp))] items-center px-[calc(21.2*var(--dp))] py-[calc(28.6*var(--dp))]">
        <span className="min-w-0 truncate">Auto-deselect Eraser</span>
        <AutoDeselectHint units={units} />
        {/* Only the switch toggles, as in stock; the label is text. */}
        <button
          type="button"
          role="switch"
          aria-checked={autoDeselect}
          aria-label="Auto-deselect Eraser"
          onClick={() => setAutoDeselect(!autoDeselect)}
          className={cn(
            "ml-auto flex h-[calc(26.7*var(--dp))] w-[calc(53.3*var(--dp))] shrink-0 items-center justify-between rounded-[calc(2.6*var(--dp))] border-[length:calc(2.2*var(--dp))] border-popover-foreground px-[calc(3.3*var(--dp))] text-[length:calc(14*var(--dp))] leading-none font-bold",
            autoDeselect
              ? "bg-popover-foreground text-popover"
              : "bg-popover text-popover-foreground",
          )}
        >
          {autoDeselect && <span>ON</span>}
          <span
            aria-hidden
            className="size-[calc(15.6*var(--dp))] rounded-[calc(1.5*var(--dp))] bg-current"
          />
          {!autoDeselect && <span>OFF</span>}
        </button>
      </div>
      <Rule inset />
      <div className="px-[calc(21.2*var(--dp))] py-[calc(28.6*var(--dp))]">
        <button
          type="button"
          disabled={eraseDisabled}
          onClick={onErasePage}
          className="flex h-[calc(56.2*var(--dp))] w-full items-center justify-center gap-[calc(14.8*var(--dp))] rounded-[calc(14*var(--dpx))] border-[length:calc(2*var(--dpx))] border-current bg-popover text-[length:calc(17*var(--dp))] disabled:opacity-50"
        >
          <ErasePageIcon className="size-[calc(33.9*var(--dp))] shrink-0" />
          Erase Page
        </button>
      </div>
    </div>
  );
}

/** Stock's dividers: two device pixels, full width under the types, inset 10 dp elsewhere. */
function Rule({ inset = false, className }: { inset?: boolean; className?: string }) {
  return (
    <div
      aria-hidden
      className={cn(
        "h-[calc(2*var(--dpx))] bg-current",
        inset && "mx-[calc(10*var(--dp))]",
        className,
      )}
    />
  );
}

/** `<` and `>`: a thin rounded box with a chevron, 36 dp, one 0.05 mm step per tap. */
function StepButton({
  label,
  direction,
  onClick,
}: {
  label: string;
  direction: 1 | -1;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onClick}
      className="flex size-[calc(36*var(--dp))] shrink-0 items-center justify-center"
    >
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
        className="size-full"
      >
        <rect
          x="1.6"
          y="3.1"
          width="20.8"
          height="18.3"
          rx="2.5"
          strokeWidth={0.9}
          className="fill-popover"
        />
        <path
          d={direction < 0 ? "m14 8-4.4 4.25L14 16.5" : "m10 8 4.4 4.25L10 16.5"}
          strokeWidth={1.3}
        />
      </svg>
    </button>
  );
}

/**
 * The ⓘ next to Auto-deselect Eraser and its hint above it. The hint is a popover of its own inside
 * the menu, so a tap elsewhere in the menu closes only the hint. Its close repaints like the menu's:
 * no full refresh.
 */
function AutoDeselectHint({ units }: { units: EraserMenuUnits }) {
  return (
    <Popover fullRefreshOnClose={false}>
      <PopoverTrigger
        aria-label="About Auto-deselect Eraser"
        className="ml-[calc(10*var(--dp))] flex size-[calc(26*var(--dp))] shrink-0 items-center justify-center"
      >
        <Info aria-hidden strokeWidth={1.5} className="size-full" />
      </PopoverTrigger>
      <PopoverContent
        side="top"
        align="start"
        sideOffset={HINT_GAP_DEVICE_PX * units.dpx}
        style={eraserMenuStyle(units)}
        className="w-auto max-w-[min(calc(483*var(--dp)),calc(100vw-24px))] rounded-[calc(19*var(--dpx))] border-[length:calc(3*var(--dpx))] border-foreground p-[calc(21.2*var(--dp))] text-[length:calc(20*var(--dp))] leading-[1.17] shadow-none"
      >
        When enabled, auto-switch to last tool after erasing.
      </PopoverContent>
    </Popover>
  );
}
