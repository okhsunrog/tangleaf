import { useId, useLayoutEffect, useRef, useState } from "react";
import { Minus, Plus, RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { GraphItem } from "@/lib/api";

/** A directed, page-level neighborhood: containment blocks never become graph nodes. */
export function PageConnectionMap({
  selected,
  incoming,
  outgoing,
  onSelect,
}: {
  selected: GraphItem;
  incoming: GraphItem[];
  outgoing: GraphItem[];
  onSelect: (uuid: string) => void;
}) {
  const [zoom, setZoom] = useState(1);
  const marker = useId().replace(/:/g, "");
  const height = Math.max(280, Math.max(incoming.length, outgoing.length) * 86 + 80);
  const centerY = height / 2;
  const viewport = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = viewport.current;
    if (!element) return;
    element.scrollTop = centerY * zoom - element.clientHeight / 2;
    element.scrollLeft = 420 * zoom - element.clientWidth / 2;
  }, [selected.uuid, centerY, zoom]);
  const rows = (items: GraphItem[], x: number) =>
    items.map((item, index) => ({
      item,
      x,
      y: (height - items.length * 86) / 2 + index * 86 + 10,
    }));
  const sources = rows(incoming, 20);
  const targets = rows(outgoing, 590);
  const center = { item: selected, x: 305, y: centerY - 32 };
  return (
    <section aria-label="Connection map" className="rounded-xl border border-border bg-card">
      <div className="flex items-center gap-1 border-b border-border px-3 py-2">
        <span className="mr-auto text-xs text-muted-foreground">Arrows show link direction</span>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Zoom out"
          disabled={zoom <= 0.5}
          onClick={() => setZoom((value) => Math.max(0.5, value - 0.25))}
        >
          <Minus className="size-3.5" />
        </Button>
        <span className="w-10 text-center text-xs tabular-nums">{Math.round(zoom * 100)}%</span>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Zoom in"
          disabled={zoom >= 1.5}
          onClick={() => setZoom((value) => Math.min(1.5, value + 0.25))}
        >
          <Plus className="size-3.5" />
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label="Reset zoom" onClick={() => setZoom(1)}>
          <RotateCcw className="size-3.5" />
        </Button>
      </div>
      <div
        ref={viewport}
        className="max-h-[60vh] overflow-auto overscroll-contain"
        tabIndex={0}
        aria-label="Scrollable page connection map"
      >
        <svg
          width={840 * zoom}
          height={height * zoom}
          viewBox={`0 0 840 ${height}`}
          role="group"
          aria-label={`Connections for ${selected.label}`}
        >
          <defs>
            <marker
              id={marker}
              viewBox="0 0 10 10"
              refX="9"
              refY="5"
              markerWidth="6"
              markerHeight="6"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 10 5 L 0 10 z" className="fill-primary" />
            </marker>
          </defs>
          <text x="20" y="25" className="fill-muted-foreground text-xs">
            Links from · {incoming.length}
          </text>
          <text x="590" y="25" className="fill-muted-foreground text-xs">
            Links to · {outgoing.length}
          </text>
          {sources.map((node) => (
            <path
              key={`from-${node.item.uuid}`}
              d={`M 250 ${node.y + 32} C 278 ${node.y + 32}, 278 ${centerY}, 302 ${centerY}`}
              fill="none"
              className="stroke-primary/50"
              strokeWidth="1.5"
              markerEnd={`url(#${marker})`}
            />
          ))}
          {targets.map((node) => (
            <path
              key={`to-${node.item.uuid}`}
              d={`M 535 ${centerY} C 563 ${centerY}, 563 ${node.y + 32}, 587 ${node.y + 32}`}
              fill="none"
              className="stroke-primary/50"
              strokeWidth="1.5"
              markerEnd={`url(#${marker})`}
            />
          ))}
          {[...sources, center, ...targets].map((node) => (
            <foreignObject
              key={`${node.x}-${node.item.uuid}`}
              x={node.x}
              y={node.y}
              width="230"
              height="64"
            >
              <button
                type="button"
                aria-pressed={node.x === center.x}
                title={node.item.label}
                onClick={() => onSelect(node.item.uuid)}
                className="flex h-full w-full items-center justify-center rounded-lg border border-border bg-background px-3 text-center text-sm leading-snug break-words hover:border-primary focus-visible:outline-2 focus-visible:outline-primary aria-pressed:border-primary aria-pressed:bg-primary/10"
              >
                <span className="line-clamp-3">{node.item.label || "Untitled"}</span>
              </button>
            </foreignObject>
          ))}
        </svg>
      </div>
      <p className="border-t border-border px-3 py-2 text-xs text-muted-foreground">
        Select a page to center its neighborhood. Scroll to explore larger maps.
      </p>
    </section>
  );
}
