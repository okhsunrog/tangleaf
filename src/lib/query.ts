import { QueryClient, type Query } from "@tanstack/react-query";
import { events, type ChangeSet, type DomainEvent } from "@/lib/bindings";

const root = ["backend"] as const;

export const queryKeys = {
  root,
  pages: [...root, "pages"] as const,
  pageList: (filter: "notes" | "journals" | "all", limit: number) =>
    [...root, "pages", filter, limit] as const,
  journals: [...root, "journals"] as const,
  journal: (date: string) => [...root, "journals", date] as const,
  pageRoot: [...root, "page"] as const,
  page: (uuid: string) => [...root, "page", uuid] as const,
  pageDocumentRoot: [...root, "page-document"] as const,
  pageDocument: (uuid: string) => [...root, "page-document", uuid] as const,
  pageRenderRoot: [...root, "page-render"] as const,
  pageRender: (uuid: string) => [...root, "page-render", uuid] as const,
  blockRoot: [...root, "block"] as const,
  block: (uuid: string) => [...root, "block", uuid] as const,
  childrenRoot: [...root, "children"] as const,
  children: (containerUuid: string) => [...root, "children", containerUuid] as const,
  graphRoot: [...root, "graph"] as const,
  graph: (focusUuid?: string | null) => [...root, "graph", focusUuid ?? "all"] as const,
  backlinksRoot: [...root, "backlinks"] as const,
  backlinks: (uuid: string) => [...root, "backlinks", uuid] as const,
  attachmentsRoot: [...root, "attachments"] as const,
  attachments: (ownerUuid: string) => [...root, "attachments", ownerUuid] as const,
  attachmentImagesRoot: [...root, "attachments", "images"] as const,
  attachmentImages: (attachmentUuids: readonly string[]) =>
    [...root, "attachments", "images", ...attachmentUuids] as const,
  history: [...root, "history"] as const,
  // A local refresh signal for search results held outside the query cache.
  // This is not a persisted store or database revision.
  searchRevision: [...root, "search-revision"] as const,
  settings: [...root, "settings"] as const,
  inputCapabilities: [...root, "input-capabilities"] as const,
  einkWise: [...root, "einkwise"] as const,
  handwritingStatusRoot: [...root, "handwriting-status"] as const,
  handwritingStatus: (uuid: string) => [...root, "handwriting-status", uuid] as const,
  handwritingVersions: (pageUuid: string) => [...root, "handwriting-version", pageUuid] as const,
  handwritingVersion: (pageUuid: string, versionUuid: string) =>
    [...root, "handwriting-version", pageUuid, versionUuid] as const,
  syncStatus: [...root, "sync-status"] as const,
  serverAi: [...root, "server-ai"] as const,
};

export function createAppQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: Number.POSITIVE_INFINITY,
        retry: 1,
        refetchOnWindowFocus: false,
      },
    },
  });
}

type QuerySelection = { queryKey: readonly unknown[]; exact?: boolean };
type SnapshotPublication = { query: Query; epoch: number };
const snapshotEpochs = new WeakMap<Query, number>();

export function captureSnapshotPublication(queryClient: QueryClient, queryKey: readonly unknown[]) {
  const query = queryClient
    .getQueryCache()
    .build(queryClient, queryClient.defaultQueryOptions({ queryKey }));
  return { query, epoch: snapshotEpochs.get(query) ?? 0 } satisfies SnapshotPublication;
}

function advanceSnapshotEpoch(query: Query) {
  snapshotEpochs.set(query, (snapshotEpochs.get(query) ?? 0) + 1);
}

export function refreshQueries(queryClient: QueryClient, selection: QuerySelection) {
  // invalidateQueries reuses an in-flight first load when it has no cached data.
  // Retire that read explicitly: its SQLite snapshot may predate the commit.
  // Cancellation takes effect synchronously. Start the replacement in this same
  // turn so a subsequent event can cancel it too, without an await between them.
  for (const query of queryClient.getQueryCache().findAll(selection)) advanceSnapshotEpoch(query);
  const cancellation = queryClient.cancelQueries(selection);
  const refresh = queryClient.invalidateQueries(selection);
  return Promise.all([cancellation, refresh]).then(() => undefined);
}

export function publishQuerySnapshot<T>(
  queryClient: QueryClient,
  publication: SnapshotPublication,
  snapshot: T | ((previous: T | undefined) => T | undefined),
) {
  const queryKey = publication.query.queryKey;
  const current = queryClient.getQueryCache().find({ queryKey, exact: true });
  if (current !== publication.query || (snapshotEpochs.get(current) ?? 0) !== publication.epoch) {
    // An event or another acknowledgement superseded this IPC result. Preserve
    // the newer read, including a first load still in flight, and never recreate
    // an entry evicted by deletion merely to publish the older command result.
    // A different publication may have advanced the epoch without an event.
    // Neither acknowledgement proves which snapshot is newest, so reread an
    // idle entry even when its current data was just published successfully.
    if (current?.state.fetchStatus === "idle") {
      void queryClient.invalidateQueries({ queryKey, exact: true });
    }
    return false;
  }
  publishSnapshot(queryClient, current, snapshot);
  return true;
}

function publishSnapshot<T>(
  queryClient: QueryClient,
  query: Query,
  snapshot: T | ((previous: T | undefined) => T | undefined),
) {
  // Retire earlier reads before publishing a command acknowledgement. An IPC
  // read need not support AbortSignal: cancellation also discards its result.
  const queryKey = query.queryKey;
  advanceSnapshotEpoch(query);
  void queryClient.cancelQueries({ queryKey, exact: true });
  const published = queryClient.setQueryData<T>(queryKey, snapshot);
  // A partial cache patch cannot construct a missing snapshot. Restart a
  // canceled first load in that case instead of leaving a mounted view pending.
  if (published === undefined) {
    void queryClient.invalidateQueries({ queryKey, exact: true });
  }
}

export async function applyDomainEvent(queryClient: QueryClient, event: DomainEvent) {
  const invalidate = (queryKey: readonly unknown[]) => refreshQueries(queryClient, { queryKey });

  switch (event.kind) {
    case "mutation_committed":
      await applyCommittedChanges(queryClient, event.commit.changes);
      return;
    case "pages_changed":
      await Promise.all([
        invalidate(queryKeys.pages),
        invalidate(queryKeys.journals),
        invalidate(queryKeys.graphRoot),
        invalidate(queryKeys.backlinksRoot),
        ...event.page_uuids.map((uuid) => invalidate(queryKeys.page(uuid))),
        ...event.page_uuids.map((uuid) => invalidate(queryKeys.handwritingStatus(uuid))),
      ]);
      return;
    case "history_changed":
      await invalidate(queryKeys.history);
      return;
    case "settings_changed":
      await invalidate(queryKeys.settings);
      return;
    case "sync_status_changed":
      // Publication progress changes the unsent/conflict state of every note.
      await Promise.all([
        invalidate(queryKeys.syncStatus),
        invalidate(queryKeys.handwritingStatusRoot),
      ]);
      return;
    case "server_ai_changed":
      await invalidate(queryKeys.serverAi);
      return;
    case "workspace_changed":
      await invalidate(queryKeys.root);
  }
}

async function applyCommittedChanges(queryClient: QueryClient, changes: ChangeSet) {
  const invalidate = (queryKey: readonly unknown[]) =>
    refreshQueries(queryClient, { queryKey, exact: true });
  const invalidateFamily = (queryKey: readonly unknown[]) =>
    refreshQueries(queryClient, { queryKey });
  const remove = (queryKey: readonly unknown[]) =>
    queryClient.removeQueries({ queryKey, exact: true });
  const clearSnapshot = (queryKey: readonly unknown[], empty: null | []) => {
    const query = queryClient.getQueryCache().find({ queryKey, exact: true });
    if (!query) return;
    if (query.getObserversCount() === 0) {
      remove(queryKey);
      return;
    }
    // Eviction alone does not update mounted observers. Cancel an older read
    // before publishing absence, and retain it for mounted observers.
    publishSnapshot(queryClient, query, empty);
  };
  const removeInactiveNull = (queryKey: readonly unknown[]) => {
    const query = queryClient.getQueryCache().find({ queryKey, exact: true });
    if (query?.getObserversCount() === 0 && query.state.data === null) remove(queryKey);
  };
  const deletedPages = new Set(changes.deletedPages);
  const deletedBlocks = new Set<string>();
  const affectedPages = new Set([...changes.affectedPages, ...changes.changedPages]);
  const containers = new Set<string>();
  const changedBlocks: string[] = [];
  for (const block of changes.blocks) {
    for (const placement of [block.before, block.after]) {
      if (placement) {
        affectedPages.add(placement.pageUuid);
        containers.add(placement.parentUuid ?? placement.pageUuid);
      }
    }
    if (block.after) {
      changedBlocks.push(block.uuid);
    } else {
      deletedBlocks.add(block.uuid);
    }
  }
  for (const uuid of deletedBlocks) {
    containers.delete(uuid);
    clearSnapshot(queryKeys.block(uuid), null);
    clearSnapshot(queryKeys.children(uuid), []);
    clearSnapshot(queryKeys.attachments(uuid), []);
  }
  for (const uuid of deletedPages) {
    affectedPages.delete(uuid);
    containers.delete(uuid);
    clearSnapshot(queryKeys.page(uuid), null);
    clearSnapshot(queryKeys.pageDocument(uuid), null);
    clearSnapshot(queryKeys.pageRender(uuid), null);
    clearSnapshot(queryKeys.children(uuid), []);
    clearSnapshot(queryKeys.attachments(uuid), []);
    remove(queryKeys.handwritingStatus(uuid));
  }
  // Undo can publish absence to an open pane before it unmounts. If Redo
  // recreates that entity later, reopening must load it rather than expose the
  // inactive cached null as a successful read while refetching in the background.
  for (const uuid of changes.changedPages) {
    if (!deletedPages.has(uuid)) removeInactiveNull(queryKeys.page(uuid));
  }
  for (const uuid of affectedPages) {
    removeInactiveNull(queryKeys.pageDocument(uuid));
    removeInactiveNull(queryKeys.pageRender(uuid));
  }
  for (const uuid of changedBlocks) removeInactiveNull(queryKeys.block(uuid));

  const invalidations = [
    ...[...affectedPages].flatMap((uuid) => [
      invalidate(queryKeys.pageDocument(uuid)),
      invalidate(queryKeys.pageRender(uuid)),
    ]),
    ...changedBlocks.map((uuid) => invalidate(queryKeys.block(uuid))),
    ...[...containers].map((uuid) => invalidate(queryKeys.children(uuid))),
    ...changes.changedPages
      .filter((uuid) => !deletedPages.has(uuid))
      .flatMap((uuid) => [
        invalidate(queryKeys.page(uuid)),
        invalidate(queryKeys.handwritingStatus(uuid)),
      ]),
    ...changes.attachmentOwners
      .filter((uuid) => !deletedPages.has(uuid) && !deletedBlocks.has(uuid))
      .map((uuid) => invalidate(queryKeys.attachments(uuid))),
    ...(changes.handwritingPages ?? [])
      .filter((uuid) => !deletedPages.has(uuid))
      .flatMap((uuid) => [
        invalidate(queryKeys.handwritingStatus(uuid)),
        invalidateFamily(queryKeys.handwritingVersions(uuid)),
      ]),
  ];
  if (changes.changedPages.length || changes.deletedPages.length) {
    invalidations.push(invalidateFamily(queryKeys.pages), invalidateFamily(queryKeys.journals));
  }
  if (changes.graphChangedContent.length) {
    invalidations.push(invalidateFamily(queryKeys.graphRoot));
  }
  // Backlink results embed complete source blocks, including Markdown and
  // revisions, even when the graph's labels and edge sets stay unchanged.
  if (changes.blocks.length || changes.graphChangedContent.length) {
    invalidations.push(invalidateFamily(queryKeys.backlinksRoot));
  }
  if (changes.attachmentOwners.length) {
    invalidations.push(invalidateFamily(queryKeys.attachmentImagesRoot));
  }
  if (changes.historyChanged) invalidations.push(invalidate(queryKeys.history));
  if (changes.blocks.length || changes.changedPages.length || changes.deletedPages.length) {
    queryClient.setQueryData<number>(queryKeys.searchRevision, (revision = 0) => revision + 1);
  }
  await Promise.all(invalidations);
}

export function listenForDomainEvents(queryClient: QueryClient) {
  return events.domainEvent.listen(({ payload }) => {
    void applyDomainEvent(queryClient, payload);
  });
}
