import type { ComponentProps } from "react";
import type { EraserMode } from "./ink-editing";

/**
 * The eraser icons, drawn for this app in the lucide style (24-unit box, 2-unit round stroke,
 * `currentColor`), after the shapes stock Notes uses: the same tilted block eraser on a baseline
 * for the pixel eraser, inside a dashed ring for the lasso eraser, and over a squiggle for the
 * stroke eraser.
 */

/**
 * A block eraser tilted 45°, its bottom edge on y = 21, with the line that splits off its tip:
 * lucide's own `eraser` glyph without its baseline, so the pixel eraser is exactly that icon.
 */
const ERASER_BODY =
  "M12.834 21H8a2 2 0 0 1-1.42-.587l-3.994-3.999a2 2 0 0 1 0-2.828l10-10a2 2 0 0 1 2.828 0l5.999 6a2 2 0 0 1 0 2.828L12.834 21";
const ERASER_SPLIT = "m5.082 11.09 8.828 8.828";

type IconProps = ComponentProps<"svg">;

function Icon({ children, ...props }: IconProps) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width={24}
      height={24}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      {...props}
    >
      {children}
    </svg>
  );
}

export function PixelEraserIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d={ERASER_BODY} />
      <path d={ERASER_SPLIT} />
      <path d="M12.834 21H21" />
    </Icon>
  );
}

export function LassoEraserIcon(props: IconProps) {
  return (
    <Icon {...props}>
      {/* Sixteen dashes around the loop; butt ends, or the dashes would run into each other. */}
      <circle
        cx="12"
        cy="12"
        r="10.5"
        strokeWidth={1.5}
        strokeLinecap="butt"
        strokeDasharray="2.12 2"
      />
      <g transform="translate(12 12) scale(0.52) translate(-12 -12.2)" strokeWidth={3}>
        <path d={ERASER_BODY} />
        <path d={ERASER_SPLIT} />
      </g>
    </Icon>
  );
}

export function StrokeEraserIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d={ERASER_BODY} />
      <path d={ERASER_SPLIT} />
      <path d="M15.5 20.8c.8-1.5 1.8-1.5 2.4 0s1.6 1.5 2.4 0" />
    </Icon>
  );
}

/** Erase Page: the eraser resting on one flat sheet. */
export function ErasePageIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M2 18.5 8 15h14l-6 3.5Z" />
      <g transform="translate(13.2 9.6) scale(0.6) translate(-12 -12)" strokeWidth={3.2}>
        <path d={ERASER_BODY} />
        <path d={ERASER_SPLIT} />
      </g>
    </Icon>
  );
}

const ICONS: Record<EraserMode, (props: IconProps) => React.JSX.Element> = {
  pixel: PixelEraserIcon,
  lasso: LassoEraserIcon,
  stroke: StrokeEraserIcon,
};

/** The icon of one eraser type; the toolbar's eraser button shows the current type's. */
export function EraserTypeIcon({ type, ...props }: IconProps & { type: EraserMode }) {
  const Glyph = ICONS[type];
  return <Glyph data-eraser-type={type} {...props} />;
}
