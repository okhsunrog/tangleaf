export type NoteSearchMatch = {
  blockUuid: string | null;
  from: number;
  to: number;
  occurrence: number;
};

/** Literal Unicode search: punctuation is never treated as a regular expression. */
export function findTextMatches(
  text: string,
  query: string,
  matchCase: boolean,
): { from: number; to: number }[] {
  if (!query) return [];
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(escaped, matchCase ? "gu" : "giu");
  return Array.from(text.matchAll(pattern), (match) => ({
    from: match.index,
    to: match.index + match[0].length,
  }));
}
