import { useQueries, useQuery } from "@tanstack/react-query";
import { findBacklinks, getPage, contentUuid } from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { pageDisplayTitle } from "@/features/journal/journal-date";
import { useWorkspaceController } from "@/features/workspace/workspace-controller";
import { dispositionFromShiftKey } from "@/features/workspace/workspace-model";

export function PageBacklinks({ pageUuid }: { pageUuid: string }) {
  const controller = useWorkspaceController();
  const links = useQuery({
    queryKey: queryKeys.backlinks(pageUuid),
    queryFn: () => findBacklinks(pageUuid),
  });
  const pageUuids = [
    ...new Set(
      (links.data ?? []).map((link) =>
        link.kind === "block" ? link.record.pageUuid : link.record.uuid,
      ),
    ),
  ];
  const sources = useQueries({
    queries: pageUuids.map((uuid) => ({
      queryKey: queryKeys.page(uuid),
      queryFn: () => getPage(uuid),
    })),
  });
  const pages = new Map(
    sources.flatMap((source) => (source.data ? [[source.data.uuid, source.data] as const] : [])),
  );
  return (
    <section aria-label="Backlinks" className="mt-10 border-t border-border/60 pt-5">
      <h2 className="text-sm font-semibold">
        Backlinks{" "}
        {links.data && <span className="text-muted-foreground">({links.data.length})</span>}
      </h2>
      {links.isPending ? (
        <p className="mt-3 text-sm text-muted-foreground">Loading backlinks…</p>
      ) : links.isError ? (
        <p role="alert" className="mt-3 text-sm text-destructive">
          Could not load backlinks.{" "}
          <button type="button" onClick={() => void links.refetch()}>
            Retry
          </button>
        </p>
      ) : links.data.length === 0 ? (
        <p className="mt-3 text-sm text-muted-foreground">No links to this page yet.</p>
      ) : (
        <ul className="mt-3 space-y-2">
          {links.data.map((link) => {
            const source = pages.get(
              link.kind === "block" ? link.record.pageUuid : link.record.uuid,
            );
            return (
              <li key={contentUuid(link)}>
                <button
                  type="button"
                  className="w-full rounded-lg border border-border/60 p-3 text-left hover:bg-accent focus-visible:outline-2 focus-visible:outline-primary"
                  onClick={(event) =>
                    void controller.openContent(link, dispositionFromShiftKey(event.shiftKey))
                  }
                >
                  <span className="block text-sm font-medium">
                    {source ? pageDisplayTitle(source) : "Loading source page…"}
                  </span>
                  {link.kind === "block" && (
                    <span className="mt-1 line-clamp-3 block text-sm break-words text-muted-foreground">
                      {link.record.markdown.replace(/\[\[([^\]]+)\]\]/g, "$1") || "Empty block"}
                    </span>
                  )}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
