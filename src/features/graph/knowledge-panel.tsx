import { PageConnectionMap } from "./page-connection-map";
import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, ArrowRight, RefreshCw, Sparkles, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ChatCard } from "@/features/chat/chat-card";
import type { AssistantController } from "@/features/chat/use-assistant-controller";
import type { MarkdownOpenHandler } from "@/features/markdown";
import { getGraphSnapshot, getPage, type Content, type Page } from "@/lib/api";
import { queryKeys } from "@/lib/query";
import {
  dispositionFromShiftKey,
  type OpenDisposition,
} from "@/features/workspace/workspace-model";
import { notifyError } from "@/lib/notify";

type PanelProps = {
  page: Page | null;
  onOpenMarkdownLink: MarkdownOpenHandler;
  controller: AssistantController;
};
type GraphProps = {
  page: Page | null;
  onOpenContent: (content: Content, disposition?: OpenDisposition) => void | Promise<void>;
};

export function KnowledgePanel({ page, onOpenMarkdownLink, controller }: PanelProps) {
  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div className="flex shrink-0 items-center gap-2 px-1 pt-1">
        <span className="flex size-7 items-center justify-center rounded-xl bg-primary/10 text-primary">
          <Sparkles className="size-3.5" />
        </span>
        <div>
          <p className="text-xs font-semibold">Knowledge companion</p>
          <p className="text-[10px] eink:text-xs text-muted-foreground">Context-aware tools</p>
        </div>
      </div>
      <div className="min-h-0 flex-1 pt-1">
        <ChatCard page={page} onOpenMarkdownLink={onOpenMarkdownLink} controller={controller} />
      </div>
    </div>
  );
}

export function GraphWorkspace({
  page,
  onOpenContent,
  onClose,
}: GraphProps & { onClose: () => void }) {
  const [selectedUuid, setSelectedUuid] = useState<string | null>(page?.uuid ?? null);
  const [filter, setFilter] = useState("");
  const [view, setView] = useState<"list" | "map">("list");
  const graph = useQuery({
    queryKey: queryKeys.graph(null),
    queryFn: () => getGraphSnapshot(null),
  });
  const items = graph.data?.items ?? [];
  const edges = graph.data?.edges ?? [];
  const degrees = useMemo(() => {
    const neighbors = new Map<string, Set<string>>();
    for (const edge of graph.data?.edges ?? []) {
      if (edge.sourceUuid === edge.targetUuid) continue;
      for (const [from, to] of [
        [edge.sourceUuid, edge.targetUuid],
        [edge.targetUuid, edge.sourceUuid],
      ]) {
        if (!neighbors.has(from)) neighbors.set(from, new Set());
        neighbors.get(from)!.add(to);
      }
    }
    return neighbors;
  }, [graph.data]);
  const visible = items
    .filter((item) => item.label.toLocaleLowerCase().includes(filter.toLocaleLowerCase()))
    .sort(
      (a, b) =>
        (degrees.get(b.uuid)?.size ?? 0) - (degrees.get(a.uuid)?.size ?? 0) ||
        a.label.localeCompare(b.label),
    );
  const selected = items.find((item) => item.uuid === selectedUuid);
  const incoming = new Set(
    edges.filter((edge) => edge.targetUuid === selectedUuid).map((edge) => edge.sourceUuid),
  );
  const outgoing = new Set(
    edges.filter((edge) => edge.sourceUuid === selectedUuid).map((edge) => edge.targetUuid),
  );
  const openPage = async (uuid: string, shiftKey: boolean) => {
    try {
      const record = await getPage(uuid);
      if (record) await onOpenContent({ kind: "page", record }, dispositionFromShiftKey(shiftKey));
    } catch (error) {
      notifyError("Could not open page", error);
    }
  };
  const connections = (direction: "incoming" | "outgoing") => {
    const targets = direction === "incoming" ? incoming : outgoing;
    const connected = items
      .filter((item) => targets.has(item.uuid))
      .sort((a, b) => a.label.localeCompare(b.label));
    return (
      <section className="min-w-0 flex-1">
        <h3 className="mb-3 flex items-center gap-2 text-xs font-semibold text-muted-foreground">
          {direction === "incoming" ? "Links from" : "Links to"} · {connected.length}
          <ArrowRight className="size-3.5" />
        </h3>
        {connected.length === 0 && (
          <p className="rounded-xl border border-dashed border-border p-4 text-sm text-muted-foreground">
            No {direction} links
          </p>
        )}
        <ul className="space-y-2">
          {connected.map((item) => (
            <li key={item.uuid} className="rounded-xl border border-border bg-card p-3">
              <button
                type="button"
                className="w-full text-left text-sm font-medium break-words hover:text-primary"
                onClick={() => setSelectedUuid(item.uuid)}
              >
                {item.label || "Untitled"}
              </button>
              <button
                type="button"
                className="mt-2 text-xs text-muted-foreground underline underline-offset-4 hover:text-primary"
                onClick={(event) => void openPage(item.uuid, event.shiftKey)}
              >
                Open page
              </button>
            </li>
          ))}
        </ul>
      </section>
    );
  };
  return (
    <div className="graph-workspace flex h-full min-h-0 flex-col bg-canvas">
      <header className="flex shrink-0 items-center gap-3 border-b border-border/60 p-4">
        <Button variant="ghost" size="icon-sm" aria-label="Back to notes" onClick={onClose}>
          <ArrowLeft className="size-4" />
        </Button>
        <div>
          <h2 className="text-sm font-semibold">Page connections</h2>
          <p className="text-xs text-muted-foreground">
            {items.length} pages · Links and block references between pages
          </p>
        </div>
        <Button
          className="ml-auto"
          variant="ghost"
          size="icon-sm"
          aria-label="Refresh graph"
          onClick={() => void graph.refetch()}
        >
          <RefreshCw className="size-4" />
        </Button>
      </header>
      {graph.isError ? (
        <p role="alert" className="p-5 text-sm text-destructive">
          Could not load connections. Use Refresh to retry.
        </p>
      ) : graph.isPending ? (
        <p role="status" className="p-5 text-sm text-muted-foreground">
          Loading connections…
        </p>
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-1 overflow-y-auto md:grid-cols-[15rem_minmax(0,1fr)] md:overflow-hidden">
          <aside className="border-b border-border p-4 md:overflow-y-auto md:border-r md:border-b-0">
            <label
              className="mb-2 flex items-center gap-2 text-xs font-medium"
              htmlFor="graph-filter"
            >
              <Search className="size-3.5" />
              Find a page
            </label>
            <Input
              id="graph-filter"
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              placeholder="Page title…"
            />
            <p className="my-3 text-xs text-muted-foreground">
              Pages sorted by number of connections
            </p>
            <ul className="max-h-52 space-y-1 overflow-y-auto md:max-h-none">
              {visible.map((item) => (
                <li key={item.uuid}>
                  <button
                    type="button"
                    aria-pressed={item.uuid === selectedUuid}
                    className="flex w-full items-start gap-2 rounded-lg px-3 py-2 text-left text-sm hover:bg-accent aria-pressed:bg-primary/10 aria-pressed:text-primary"
                    onClick={() => setSelectedUuid(item.uuid)}
                  >
                    <span className="min-w-0 flex-1 break-words">{item.label || "Untitled"}</span>
                    <span className="text-xs text-muted-foreground">
                      {degrees.get(item.uuid)?.size ?? 0}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
            {visible.length === 0 && (
              <p className="py-4 text-sm text-muted-foreground">
                {items.length ? "No matching pages." : "Create a page to get started."}
              </p>
            )}
          </aside>
          <main className="p-5 md:overflow-y-auto">
            {selected ? (
              <>
                <div className="mb-6 rounded-xl border border-primary/30 bg-primary/5 p-5">
                  <p className="mb-2 text-xs text-muted-foreground">Selected page</p>
                  <h3 className="text-xl font-semibold break-words">
                    {selected.label || "Untitled"}
                  </h3>
                  <Button
                    className="mt-3"
                    variant="outline"
                    size="sm"
                    onClick={(event) => void openPage(selected.uuid, event.shiftKey)}
                  >
                    Open page
                    <ArrowRight className="size-3.5" />
                  </Button>
                </div>
                <div className="mb-3 flex gap-1" aria-label="Connection view">
                  <Button
                    variant={view === "list" ? "secondary" : "ghost"}
                    size="sm"
                    aria-pressed={view === "list"}
                    onClick={() => setView("list")}
                  >
                    List
                  </Button>
                  <Button
                    variant={view === "map" ? "secondary" : "ghost"}
                    size="sm"
                    aria-pressed={view === "map"}
                    onClick={() => setView("map")}
                  >
                    Map
                  </Button>
                </div>
                {view === "map" ? (
                  <PageConnectionMap
                    selected={selected}
                    incoming={items.filter((item) => incoming.has(item.uuid))}
                    outgoing={items.filter((item) => outgoing.has(item.uuid))}
                    onSelect={setSelectedUuid}
                  />
                ) : (
                  <div className="flex flex-col gap-6 xl:flex-row">
                    {connections("incoming")}
                    {connections("outgoing")}
                  </div>
                )}
                <p className="mt-6 text-xs text-muted-foreground">
                  Select a linked title to explore its connections. Shift + Open page opens it
                  alongside your note.
                </p>
              </>
            ) : (
              <div className="py-12 text-center">
                <h3 className="text-lg font-medium">Explore connections</h3>
                <p className="mt-2 text-sm text-muted-foreground">
                  Choose a page to see where its links come from and where they lead.
                </p>
              </div>
            )}
          </main>
        </div>
      )}
    </div>
  );
}
