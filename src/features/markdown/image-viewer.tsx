import { useCallback, useEffect, useRef, useState } from "react";
import { registerBackOverlay } from "@/lib/back-overlays";
import { ImageOff, Maximize2, RotateCcw, ZoomIn, ZoomOut } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import type { MarkdownResolvedImage } from "./image-policy";

type Point = { x: number; y: number };
type Transform = { scale: number; x: number; y: number };

const MIN_SCALE = 1;
const MAX_SCALE = 8;

interface ImageViewerProps {
  alt: string;
  image: MarkdownResolvedImage;
  inline?: boolean;
  title?: string;
}

export function MarkdownImageViewer(props: ImageViewerProps) {
  // A changed attachment must not inherit a previous image's loaded/error state.
  return <ImageViewer key={`${props.image.src}\n${props.image.originalSrc}`} {...props} />;
}

function ImageViewer({ alt, image, inline = false, title }: ImageViewerProps) {
  const [open, setOpen] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [previewFailed, setPreviewFailed] = useState(false);
  const [originalFailed, setOriginalFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [transform, setTransform] = useState<Transform>({ scale: 1, x: 0, y: 0 });
  const pointers = useRef(new Map<number, Point>());
  const gesture = useRef<{
    distance: number;
    midpoint: Point;
    transform: Transform;
  } | null>(null);

  const reset = () => setTransform({ scale: 1, x: 0, y: 0 });
  const changeOpen = useCallback((next: boolean) => {
    setOpen(next);
    setLoaded(false);
    setOriginalFailed(false);
    if (next) setPreviewFailed(false);
    setTransform({ scale: 1, x: 0, y: 0 });
    pointers.current.clear();
    gesture.current = null;
  }, []);
  useEffect(() => {
    if (open) return registerBackOverlay(() => changeOpen(false));
  }, [open, changeOpen]);
  const zoom = (factor: number) => {
    setTransform((current) => {
      const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, current.scale * factor));
      return scale === MIN_SCALE ? { scale, x: 0, y: 0 } : { ...current, scale };
    });
  };

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogTrigger
        type="button"
        className={`markdown-image-trigger group relative max-w-full cursor-zoom-in overflow-hidden rounded-xl text-left ${inline ? "inline-flex align-middle" : "block"}`}
        aria-label={alt ? `Open image: ${alt}` : "Open image viewer"}
        onClick={(event) => event.stopPropagation()}
      >
        {previewFailed ? (
          <span className="flex min-h-28 max-w-full items-center gap-3 rounded-xl border border-border bg-muted/40 px-5 py-4 text-muted-foreground">
            <ImageOff className="size-5 shrink-0" aria-hidden="true" />
            <span className="min-w-0 text-sm">
              <span className="block font-medium text-foreground">Preview unavailable</span>
              {alt && <span className="block truncate">{alt}</span>}
              <span className="block text-xs">Open image to view or retry</span>
            </span>
          </span>
        ) : (
          <img
            key={attempt}
            alt={alt}
            crossOrigin="anonymous"
            decoding="async"
            draggable={false}
            height={image.height}
            loading="lazy"
            onError={() => setPreviewFailed(true)}
            referrerPolicy="no-referrer"
            src={image.src}
            title={title}
            width={image.width}
          />
        )}
        {!previewFailed && (
          <span className="reveal-on-hover pointer-events-none absolute right-2 bottom-2 flex size-8 items-center justify-center rounded-lg bg-black/65 text-white shadow-sm surface-blur transition">
            <Maximize2 className="size-4" />
          </span>
        )}
      </DialogTrigger>
      <DialogContent
        data-fullscreen="true"
        showCloseButton={false}
        className="inset-0 top-0 left-0 h-dvh w-dvw max-w-none translate-x-0 translate-y-0 gap-0 overflow-hidden rounded-none border-0 bg-black p-0 sm:max-w-none"
        onClick={(event) => event.stopPropagation()}
      >
        <DialogTitle className="sr-only">{alt || title || "Image viewer"}</DialogTitle>
        <DialogDescription className="sr-only">
          Full-resolution image. Use the controls to zoom, reset, or close.
        </DialogDescription>
        <div className="absolute top-[max(0.75rem,var(--safe-area-inset-top))] right-[max(0.75rem,var(--safe-area-inset-right))] left-[max(0.75rem,var(--safe-area-inset-left))] z-20 flex items-center gap-2">
          <div className="flex items-center rounded-xl border border-white/15 bg-black/60 p-1 text-white shadow-panel surface-blur">
            <ViewerButton
              label="Zoom out"
              onClick={() => zoom(1 / 1.25)}
              disabled={transform.scale <= MIN_SCALE}
            >
              <ZoomOut />
            </ViewerButton>
            <span className="w-14 text-center text-xs tabular-nums">
              {Math.round(transform.scale * 100)}%
            </span>
            <ViewerButton
              label="Zoom in"
              onClick={() => zoom(1.25)}
              disabled={transform.scale >= MAX_SCALE}
            >
              <ZoomIn />
            </ViewerButton>
            <ViewerButton label="Reset zoom" onClick={reset} disabled={transform.scale === 1}>
              <RotateCcw />
            </ViewerButton>
          </div>
          {(title || alt) && (
            <span className="min-w-0 flex-1 truncate px-2 text-center text-sm text-white/75">
              {title || alt}
            </span>
          )}
          <button
            type="button"
            aria-label="Close image viewer"
            className="ml-auto flex size-10 items-center justify-center rounded-xl border border-white/15 bg-black/60 text-xl text-white shadow-panel surface-blur hover:bg-white/15"
            onClick={() => changeOpen(false)}
          >
            ×
          </button>
        </div>
        {originalFailed && (
          <div
            role="status"
            className="absolute right-4 bottom-[max(1rem,var(--safe-area-inset-bottom))] left-4 z-20 mx-auto flex w-fit max-w-[calc(100%_-_2rem)] flex-wrap items-center justify-center gap-3 rounded-xl border border-white/20 bg-black/85 px-4 py-3 text-sm text-white shadow-panel"
          >
            <span>Could not load the full image.</span>
            <button
              type="button"
              className="rounded-lg border border-white/25 px-3 py-2 hover:bg-white/15 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
              onClick={() => {
                setLoaded(false);
                setOriginalFailed(false);
                setPreviewFailed(false);
                setAttempt((current) => current + 1);
              }}
            >
              Retry
            </button>
          </div>
        )}
        <div
          className="relative flex h-full w-full touch-none select-none items-center justify-center overflow-hidden"
          onDoubleClick={() => (transform.scale === 1 ? zoom(2) : reset())}
          onWheel={(event) => {
            event.preventDefault();
            zoom(event.deltaY < 0 ? 1.18 : 1 / 1.18);
          }}
          onPointerDown={(event) => {
            event.currentTarget.setPointerCapture(event.pointerId);
            pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
            gesture.current = gestureState(pointers.current, transform);
          }}
          onPointerMove={(event) => {
            if (!pointers.current.has(event.pointerId)) return;
            const previous = pointers.current.get(event.pointerId);
            pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
            if (pointers.current.size === 1 && previous && transform.scale > 1) {
              setTransform((current) => ({
                ...current,
                x: current.x + event.clientX - previous.x,
                y: current.y + event.clientY - previous.y,
              }));
              return;
            }
            const initial = gesture.current;
            const current = gestureState(pointers.current, transform);
            if (!initial || !current || pointers.current.size < 2) return;
            const scale = Math.min(
              MAX_SCALE,
              Math.max(MIN_SCALE, initial.transform.scale * (current.distance / initial.distance)),
            );
            setTransform({
              scale,
              x: initial.transform.x + current.midpoint.x - initial.midpoint.x,
              y: initial.transform.y + current.midpoint.y - initial.midpoint.y,
            });
          }}
          onPointerUp={(event) => {
            pointers.current.delete(event.pointerId);
            gesture.current = gestureState(pointers.current, transform);
          }}
          onPointerCancel={(event) => {
            pointers.current.delete(event.pointerId);
            gesture.current = gestureState(pointers.current, transform);
          }}
        >
          {!loaded && !originalFailed && previewFailed && (
            <span role="status" className="absolute text-sm text-white/70">
              Loading image…
            </span>
          )}
          {!loaded && !previewFailed && (
            <div className="absolute inset-0 flex items-center justify-center">
              <img
                key={attempt}
                aria-hidden="true"
                crossOrigin="anonymous"
                className="max-h-full max-w-full object-contain"
                draggable={false}
                onError={() => setPreviewFailed(true)}
                referrerPolicy="no-referrer"
                src={image.src}
              />
            </div>
          )}
          <img
            key={attempt}
            alt={alt}
            crossOrigin="anonymous"
            className="max-h-full max-w-full object-contain will-change-transform"
            decoding="async"
            draggable={false}
            onError={() => {
              setLoaded(false);
              setOriginalFailed(true);
            }}
            onLoad={async (event) => {
              const element = event.currentTarget;
              try {
                await element.decode();
                // A closing dialog can unmount while decoding is in flight.
                if (element.isConnected) {
                  setLoaded(true);
                  setOriginalFailed(false);
                }
              } catch {
                // A failed original keeps the preview and exposes an explicit retry.
                if (element.isConnected) setOriginalFailed(true);
              }
            }}
            referrerPolicy="no-referrer"
            src={open ? image.originalSrc : undefined}
            style={{
              visibility: loaded ? "visible" : "hidden",
              transform: `translate3d(${transform.x}px, ${transform.y}px, 0) scale(${transform.scale})`,
            }}
          />
        </div>
      </DialogContent>
    </Dialog>
  );
}

function ViewerButton({
  children,
  disabled,
  label,
  onClick,
}: {
  children: React.ReactElement<{ className?: string }>;
  disabled?: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
      className="flex size-9 items-center justify-center rounded-lg transition hover:bg-white/15 disabled:opacity-35 [&_svg]:size-4"
    >
      {children}
    </button>
  );
}

function gestureState(pointers: ReadonlyMap<number, Point>, transform: Transform) {
  const points = [...pointers.values()];
  if (points.length === 0) return null;
  if (points.length === 1) {
    return { distance: 1, midpoint: points[0], transform };
  }
  const [first, second] = points;
  return {
    distance: Math.max(1, Math.hypot(second.x - first.x, second.y - first.y)),
    midpoint: { x: (first.x + second.x) / 2, y: (first.y + second.y) / 2 },
    transform,
  };
}
