import { CheckCircle2, ChevronRight, LoaderCircle } from "lucide-react";
import type { ToolCallView } from "@/lib/api";

const LABELS: Record<string, string> = {
  search_and_expand: "Search notes and connections",
  search_agentic: "Search notes",
  list_pages: "List notes",
  read_subtree: "Read note content",
  read_ancestors: "Read section context",
  get_content: "Read source",
  neighbors: "Find connected notes",
  find_backlinks: "Find references",
  create_page: "Create note",
};

export function ToolActivity({ tools, busy }: { tools: readonly ToolCallView[]; busy: boolean }) {
  if (tools.length === 0) return null;
  const pending = busy && tools.some((tool) => tool.result === undefined);
  const searches = tools.filter(
    (tool) => tool.name === "search_and_expand" || tool.name === "search_agentic",
  ).length;
  const searchOnly = searches === tools.length;
  const label = searchOnly
    ? `${pending ? "Searching notes" : "Searched notes"} · ${searches} ${searches === 1 ? "query" : "queries"}`
    : `${pending ? "Working with notes" : "Note activity"} · ${tools.length} ${tools.length === 1 ? "step" : "steps"}`;
  return (
    <details className="group/activity mb-3 text-xs text-muted-foreground">
      <summary className="flex w-fit cursor-pointer list-none items-center gap-1.5 rounded-sm py-1 hover:text-foreground focus-visible:outline-2 focus-visible:outline-primary [&::-webkit-details-marker]:hidden">
        {pending ? (
          <LoaderCircle aria-hidden="true" className="size-3.5 animate-spin eink:animate-none" />
        ) : (
          <ChevronRight
            aria-hidden="true"
            className="size-3.5 transition-transform group-open/activity:rotate-90 eink:transition-none"
          />
        )}
        <span aria-live="polite">{label}</span>
      </summary>
      <ul className="mt-1 space-y-2 border-l border-border/60 pl-3">
        {tools.map((tool) => (
          <li key={tool.id}>
            <div className="flex items-center gap-1.5 py-1 text-foreground">
              {tool.result !== undefined && (
                <CheckCircle2 aria-hidden="true" className="size-3 shrink-0" />
              )}
              {LABELS[tool.name] ?? "Note operation"}
            </div>
            <pre className="max-h-32 overflow-auto rounded-md bg-muted/60 p-2 text-[11px] whitespace-pre-wrap break-all">
              {tool.name}({JSON.stringify(tool.args, null, 2)})
            </pre>
          </li>
        ))}
      </ul>
    </details>
  );
}
