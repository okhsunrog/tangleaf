import { createContext, useContext, useEffect, useMemo, useRef, useState } from "react";
import type { MarkdownImageResolver, MarkdownOpenHandler } from "@/features/markdown";
import type { Block, PageLayout } from "@/lib/api";
import type { EditorState } from "@codemirror/state";

type Store = {
  editorStates: Map<string, EditorState>;
  editingUuid: string | null;
  setEditing: (uuid: string | null) => void;
  layout: PageLayout;
  readOnly: boolean;
  onOpenMarkdownLink: MarkdownOpenHandler;
  resolveImage?: MarkdownImageResolver;
  hasChildren: (uuid: string) => boolean;
  isCollapsed: (uuid: string) => boolean;
  toggleCollapsed: (uuid: string) => void;
};

const OutlinerCtx = createContext<Store | null>(null);

export function useOutliner() {
  const ctx = useContext(OutlinerCtx);
  if (!ctx) throw new Error("useOutliner must be used inside <OutlinerProvider>");
  return ctx;
}

/** Only ephemeral editor state lives here. Persisted blocks and child lists are
 * owned by Rust/SQLite and cached by TanStack Query. */
export function OutlinerProvider({
  children,
  initialEditingUuid = null,
  initialEditingRequest = 0,
  navigationRequest = 0,
  navigationTargetUuid = null,
  layout,
  onOpenMarkdownLink,
  resolveImage,
  blocks,
  readOnly,
}: {
  children: React.ReactNode;
  initialEditingUuid?: string | null;
  initialEditingRequest?: number;
  navigationRequest?: number;
  navigationTargetUuid?: string | null;
  layout: PageLayout;
  onOpenMarkdownLink: MarkdownOpenHandler;
  resolveImage?: MarkdownImageResolver;
  blocks: readonly Block[];
  readOnly: boolean;
}) {
  const [editingUuid, setEditing] = useState<string | null>(
    initialEditingRequest > 0 ? initialEditingUuid : null,
  );
  const [editorStates] = useState(() => new Map<string, EditorState>());
  const [collapsedUuids, setCollapsedUuids] = useState<ReadonlySet<string>>(() => {
    const collapsed = new Set<string>();
    for (const block of blocks) {
      if (localStorage.getItem(`outliner.collapsed.${block.uuid}`) === "1") {
        collapsed.add(block.uuid);
      }
    }
    return collapsed;
  });
  const parentUuids = useMemo(
    () => new Set(blocks.flatMap((block) => (block.parentUuid ? [block.parentUuid] : []))),
    [blocks],
  );

  const revealed = useRef<{ uuid: string | null; request: number } | null>(null);
  useEffect(() => {
    if (
      revealed.current?.uuid === navigationTargetUuid &&
      revealed.current.request === navigationRequest
    )
      return;
    revealed.current = { uuid: navigationTargetUuid, request: navigationRequest };
    const parents = new Map(blocks.map((block) => [block.uuid, block.parentUuid]));
    const ancestors = new Set<string>();
    let parent = navigationTargetUuid ? parents.get(navigationTargetUuid) : null;
    while (parent && !ancestors.has(parent)) {
      ancestors.add(parent);
      parent = parents.get(parent);
    }
    setCollapsedUuids((current) => {
      const next = new Set(current);
      for (const uuid of ancestors) {
        next.delete(uuid);
        localStorage.removeItem(`outliner.collapsed.${uuid}`);
      }
      return next;
    });
  }, [blocks, navigationTargetUuid, navigationRequest]);

  useEffect(() => {
    if (initialEditingRequest > 0 && initialEditingUuid !== null) {
      setEditing(initialEditingUuid);
    }
  }, [initialEditingRequest, initialEditingUuid]);

  useEffect(() => {
    if (readOnly) setEditing(null);
  }, [readOnly]);

  const toggleCollapsed = (uuid: string) => {
    setCollapsedUuids((current) => {
      const next = new Set(current);
      if (next.delete(uuid)) localStorage.removeItem(`outliner.collapsed.${uuid}`);
      else {
        next.add(uuid);
        localStorage.setItem(`outliner.collapsed.${uuid}`, "1");
      }
      return next;
    });
  };

  const store = useMemo(
    () => ({
      editorStates,
      editingUuid,
      setEditing,
      layout,
      onOpenMarkdownLink,
      readOnly,
      resolveImage,
      hasChildren: (uuid: string) => parentUuids.has(uuid),
      isCollapsed: (uuid: string) => collapsedUuids.has(uuid),
      toggleCollapsed,
    }),
    [
      editorStates,
      collapsedUuids,
      editingUuid,
      layout,
      onOpenMarkdownLink,
      parentUuids,
      readOnly,
      resolveImage,
    ],
  );

  return <OutlinerCtx.Provider value={store}>{children}</OutlinerCtx.Provider>;
}
