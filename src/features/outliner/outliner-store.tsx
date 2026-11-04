import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { MarkdownImageResolver, MarkdownOpenHandler } from "@/features/markdown";
import type { Block, PageLayout } from "@/lib/api";
import type { EditorState } from "@codemirror/state";
import { expandBlockSelection } from "./block-selection-model";
import { flattenBlocks } from "./outliner-tree";
import { SaveAcknowledgement } from "@/features/pages/save-acknowledgement";
import type { PersistedTextSnapshot } from "@/features/pages/page-session";

type Store = {
  editorStates: Map<string, EditorState>;
  blockAcknowledgement: (uuid: string) => SaveAcknowledgement<PersistedTextSnapshot>;
  editingUuid: string | null;
  setEditing: (uuid: string | null, caret?: number) => void;
  editingCaret: number | undefined;
  previousVisibleBlock: (uuid: string) => Block | null;
  merging: boolean;
  setMerging: (merging: boolean) => void;
  selectedUuids: ReadonlySet<string>;
  selectBlocks: (uuids: readonly string[]) => void;
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
  const [editingUuid, setEditingUuid] = useState<string | null>(
    initialEditingRequest > 0 ? initialEditingUuid : null,
  );
  const [selection, selectBlocks] = useState<readonly string[]>([]);
  const [editingCaret, setEditingCaret] = useState<number>();
  const [merging, setMerging] = useState(false);
  const selectedUuids = useMemo(() => expandBlockSelection(blocks, selection), [blocks, selection]);
  const setEditing = useCallback((uuid: string | null, caret?: number) => {
    selectBlocks([]);
    setEditingCaret(caret);
    setEditingUuid(uuid);
  }, []);
  const [editorStates] = useState(() => new Map<string, EditorState>());
  const [acknowledgements] = useState(
    () => new Map<string, SaveAcknowledgement<PersistedTextSnapshot>>(),
  );
  const blockAcknowledgement = useCallback(
    (uuid: string) => {
      let handoff = acknowledgements.get(uuid);
      if (!handoff) {
        handoff = new SaveAcknowledgement<PersistedTextSnapshot>(uuid);
        acknowledgements.set(uuid, handoff);
      }
      return handoff;
    },
    [acknowledgements],
  );
  useEffect(() => {
    const existing = new Set(blocks.map((block) => block.uuid));
    for (const uuid of acknowledgements.keys())
      if (!existing.has(uuid)) acknowledgements.delete(uuid);
  }, [acknowledgements, blocks]);
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
  const visibleBlocks = useMemo(
    () => flattenBlocks(blocks, (uuid) => collapsedUuids.has(uuid)).map(({ block }) => block),
    [blocks, collapsedUuids],
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
      blockAcknowledgement,
      editingUuid,
      setEditing,
      editingCaret,
      merging,
      setMerging,
      previousVisibleBlock: (uuid: string) => {
        const index = visibleBlocks.findIndex((block) => block.uuid === uuid);
        return index > 0 ? visibleBlocks[index - 1] : null;
      },
      selectedUuids,
      selectBlocks,
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
      blockAcknowledgement,
      collapsedUuids,
      editingUuid,
      setEditing,
      editingCaret,
      merging,
      visibleBlocks,
      selectedUuids,
      layout,
      onOpenMarkdownLink,
      parentUuids,
      readOnly,
      resolveImage,
    ],
  );

  return <OutlinerCtx.Provider value={store}>{children}</OutlinerCtx.Provider>;
}
