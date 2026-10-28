import {
  MarkdownLink,
  MarkdownRenderer,
  type MarkdownOpenHandler,
  useAttachmentImageResolver,
} from "@/features/markdown";
import { useMemo } from "react";
import { ChevronRight } from "lucide-react";
import type { ChatSource } from "@/lib/api";
import { assistantCitationMarkers, prepareAssistantCitations } from "./assistant-citations";
import { useAssistantSources } from "./use-assistant-sources";

const ASSISTANT_CONTEXT = { kind: "assistant" } as const;

export default function MarkdownResponse({
  children,
  onOpenLink,
  sources,
  streaming = false,
  onSourcesResolved,
}: {
  children: string;
  onOpenLink: MarkdownOpenHandler;
  sources?: ChatSource[];
  streaming?: boolean;
  onSourcesResolved?: (sources: ChatSource[]) => void;
}) {
  const resolveImage = useAttachmentImageResolver(children);
  const markers = useMemo(() => assistantCitationMarkers(children), [children]);
  const resolved = useAssistantSources(markers, sources, streaming, onSourcesResolved);
  const prepared = prepareAssistantCitations(children, resolved.sources, markers, resolved.pending);
  return (
    <>
      <MarkdownRenderer
        className="assistant-markdown text-[13px] leading-relaxed"
        context={ASSISTANT_CONTEXT}
        markdown={prepared.markdown}
        onOpenLink={onOpenLink}
        resolveImage={resolveImage}
      />
      {prepared.citations.length > 0 && (
        <details className="group/sources mt-3 border-t border-border/60 pt-2">
          <summary className="flex w-fit cursor-pointer list-none items-center gap-1.5 rounded-sm py-1 text-xs text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-primary [&::-webkit-details-marker]:hidden">
            <ChevronRight
              aria-hidden="true"
              className="size-3.5 transition-transform group-open/sources:rotate-90 eink:transition-none"
            />
            Sources <span className="tabular-nums">· {prepared.citations.length}</span>
          </summary>
          <ol aria-label="Response sources" className="mt-1 space-y-1">
            {prepared.citations.map((source) => (
              <li key={source.target.uuid}>
                <MarkdownLink
                  href={source.href}
                  context={ASSISTANT_CONTEXT}
                  onOpenLink={onOpenLink}
                  className="flex min-w-0 gap-2 rounded-lg px-2 py-2 text-xs hover:bg-primary/5 focus-visible:outline-2 focus-visible:outline-primary"
                >
                  <span className="shrink-0 tabular-nums text-primary">[{source.number}]</span>
                  <span className="min-w-0">
                    <span className="block break-words font-medium">{source.title}</span>
                    {source.snippet && (
                      <span className="mt-0.5 line-clamp-3 block break-words leading-relaxed text-muted-foreground">
                        {source.snippet}
                      </span>
                    )}
                  </span>
                </MarkdownLink>
              </li>
            ))}
          </ol>
        </details>
      )}
    </>
  );
}
