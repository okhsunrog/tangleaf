import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { getBlock, getContainingPage, getPage, type ChatSource } from "@/lib/api";
import { pageDisplayTitle } from "@/features/journal/journal-date";
import { queryKeys } from "@/lib/query";
import type { CitationMarker } from "./assistant-citations";

/** Old histories contain only UUIDs. Resolve them locally once and retain the resulting snapshot. */
export function useAssistantSources(
  markers: readonly CitationMarker[],
  sources: ChatSource[] | undefined,
  streaming: boolean,
  onResolved?: (sources: ChatSource[]) => void,
) {
  const uuids = [...new Set(markers.map((marker) => marker.uuid))];
  const legacy = sources === undefined && uuids.length > 0;
  const query = useQuery({
    queryKey: [...queryKeys.root, "legacy-chat-sources", ...uuids],
    enabled: legacy && !streaming,
    staleTime: 30_000,
    retry: false,
    queryFn: async () => {
      const resolved = await Promise.all(
        uuids.map(async (uuid): Promise<ChatSource | null> => {
          const page = await getPage(uuid);
          if (page)
            return { target: { kind: "page", uuid }, title: pageDisplayTitle(page), snippet: "" };
          const block = await getBlock(uuid);
          if (!block) return null;
          const containingPage = await getContainingPage(uuid);
          if (!containingPage) return null;
          return {
            target: { kind: "block", uuid, pageUuid: block.pageUuid },
            title: pageDisplayTitle(containingPage),
            snippet: block.markdown.replace(/\s+/g, " ").slice(0, 280),
          };
        }),
      );
      return resolved.filter((source) => source !== null);
    },
  });

  useEffect(() => {
    // Missing notes may arrive with a later sync; don't permanently cache an incomplete migration.
    if (legacy && query.data?.length === uuids.length) onResolved?.(query.data);
  }, [legacy, onResolved, query.data, uuids.length]);

  return {
    sources: sources ?? query.data ?? [],
    pending: legacy && (streaming || query.isPending),
  };
}
