import { ChangeSet } from "@codemirror/state";

export function minimalExternalDocumentChange(previous: string, next: string): ChangeSet {
  let prefix = 0;
  const sharedLength = Math.min(previous.length, next.length);
  while (prefix < sharedLength && previous[prefix] === next[prefix]) prefix += 1;

  let suffix = 0;
  while (
    suffix < previous.length - prefix &&
    suffix < next.length - prefix &&
    previous[previous.length - suffix - 1] === next[next.length - suffix - 1]
  ) {
    suffix += 1;
  }

  return ChangeSet.of(
    {
      from: prefix,
      to: previous.length - suffix,
      insert: next.slice(prefix, next.length - suffix),
    },
    previous.length,
  );
}
