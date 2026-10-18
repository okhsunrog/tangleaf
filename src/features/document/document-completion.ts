import {
  autocompletion,
  type CompletionContext,
  type CompletionResult,
} from "@codemirror/autocomplete";
import { searchPagesByTitle, searchBlocksFts } from "@/lib/api";
import { pageToItem, blockToItem } from "@/features/outliner/autocomplete-menu";

export async function completeNoteLink(
  context: CompletionContext,
): Promise<CompletionResult | null> {
  if (context.state.readOnly) return null;
  const match = context.matchBefore(/(?:\[\[|\(\()[^\])\n]*$/);
  if (!match) return null;
  const page = match.text.startsWith("[[");
  const query = match.text.slice(2);
  try {
    const items = page
      ? (await searchPagesByTitle(query, 8)).map(pageToItem)
      : query
        ? (await searchBlocksFts(query, 8, "prefix")).map(blockToItem)
        : [];
    if (context.aborted) return null;
    const close = page ? "]]" : "))";
    return {
      from: match.from,
      to:
        context.state.sliceDoc(context.pos, context.pos + 2) === close
          ? context.pos + 2
          : context.pos,
      options: items.map((item) => ({
        label: item.label,
        detail: item.hint,
        type: page ? "text" : "reference",
        apply: `${page ? "[[" : "(("}${item.label}${close}`,
      })),
      filter: false,
    };
  } catch {
    return null;
  }
}

export const documentLinkCompletion = autocompletion({
  override: [completeNoteLink],
  activateOnTypingDelay: 120,
  defaultKeymap: true,
});
