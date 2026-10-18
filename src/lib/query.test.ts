import { describe, expect, it } from "vite-plus/test";
import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { applyDomainEvent, queryKeys } from "./query";
import type { ChangeSet, DomainEvent } from "./bindings";

describe("domain event query invalidation", () => {
  it("refreshes a committed Split and its backlinks while preserving a stable graph and unrelated pages", async () => {
    const client = new QueryClient();
    const target = [
      queryKeys.block("source"),
      queryKeys.children("page-a"),
      queryKeys.pageDocument("page-a"),
      queryKeys.pageRender("page-a"),
      queryKeys.backlinks("referenced-page"),
      queryKeys.history,
    ];
    const unrelated = [
      queryKeys.block("other"),
      queryKeys.children("page-b"),
      queryKeys.pageDocument("page-b"),
      queryKeys.pageRender("page-b"),
      queryKeys.page("page-a"),
      queryKeys.pageList("notes", 10_000),
      queryKeys.journals,
      queryKeys.graph(null),
      queryKeys.attachments("source"),
      queryKeys.settings,
    ];
    for (const key of [...target, ...unrelated]) client.setQueryData(key, {});

    await applyDomainEvent(
      client,
      committed({
        affectedPages: ["page-a"],
        blocks: [
          { uuid: "source", before: placement("page-a"), after: placement("page-a") },
          { uuid: "new", before: null, after: placement("page-a") },
        ],
        historyChanged: true,
      }),
    );

    for (const key of target) expect(client.getQueryState(key)?.isInvalidated).toBe(true);
    for (const key of unrelated) expect(client.getQueryState(key)?.isInvalidated).toBe(false);
    expect(client.getQueryData(queryKeys.searchRevision)).toBe(1);
  });

  it("refreshes both containers and documents after a committed cross-page move", async () => {
    const client = new QueryClient();
    const affected = [
      queryKeys.block("moved"),
      queryKeys.children("old-parent"),
      queryKeys.children("page-b"),
      queryKeys.pageDocument("page-a"),
      queryKeys.pageDocument("page-b"),
      queryKeys.pageRender("page-a"),
      queryKeys.pageRender("page-b"),
    ];
    const unrelated = [queryKeys.children("moved"), queryKeys.pageDocument("page-c")];
    for (const key of [...affected, ...unrelated]) client.setQueryData(key, {});

    await applyDomainEvent(
      client,
      committed({
        blocks: [
          {
            uuid: "moved",
            before: placement("page-a", "old-parent"),
            after: placement("page-b"),
          },
        ],
      }),
    );

    for (const key of affected) expect(client.getQueryState(key)?.isInvalidated).toBe(true);
    for (const key of unrelated) expect(client.getQueryState(key)?.isInvalidated).toBe(false);
  });

  it("removes deleted page and block snapshots and refreshes graph and attachments when changed", async () => {
    const client = new QueryClient();
    const removed = [
      queryKeys.page("deleted-page"),
      queryKeys.pageDocument("deleted-page"),
      queryKeys.pageRender("deleted-page"),
      queryKeys.children("deleted-page"),
      queryKeys.handwritingStatus("deleted-page"),
      queryKeys.block("deleted-block"),
      queryKeys.children("deleted-block"),
      queryKeys.attachments("deleted-block"),
    ];
    const affected = [
      queryKeys.children("page-a"),
      queryKeys.pageDocument("page-a"),
      queryKeys.pageRender("page-a"),
      queryKeys.pageList("all", 100),
      queryKeys.graph(null),
      queryKeys.backlinks("page-a"),
      queryKeys.attachments("page-a"),
      queryKeys.attachmentImages(["image-a"]),
    ];
    for (const key of [...removed, ...affected]) client.setQueryData(key, {});
    client.setQueryData(queryKeys.attachments("page-b"), {});

    await applyDomainEvent(
      client,
      committed({
        deletedPages: ["deleted-page"],
        affectedPages: ["page-a", "deleted-page"],
        blocks: [{ uuid: "deleted-block", before: placement("page-a"), after: null }],
        graphChangedContent: ["deleted-page", "deleted-block"],
        attachmentOwners: ["page-a"],
      }),
    );

    for (const key of removed) expect(client.getQueryData(key)).toBeUndefined();
    for (const key of affected) expect(client.getQueryState(key)?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.attachments("page-b"))?.isInvalidated).toBe(false);
  });

  it.each([
    ["page", queryKeys.page("deleted-page")],
    ["document", queryKeys.pageDocument("deleted-page")],
    ["render snapshot", queryKeys.pageRender("deleted-page")],
    ["block", queryKeys.block("deleted-block")],
  ])("publishes absence to a mounted %s observer on committed deletion", async (_name, key) => {
    const client = new QueryClient();
    client.setQueryData(key, { uuid: "old-snapshot" });
    const observer = new QueryObserver<{ uuid: string } | null>(client, {
      queryKey: key,
      queryFn: async () => null,
      staleTime: Number.POSITIVE_INFINITY,
    });
    const observed: unknown[] = [];
    const unsubscribe = observer.subscribe((result) => observed.push(result.data));
    try {
      await applyDomainEvent(
        client,
        committed({
          deletedPages: ["deleted-page"],
          blocks: [{ uuid: "deleted-block", before: placement("deleted-page"), after: null }],
        }),
      );

      expect(observer.getCurrentResult().data).toBeNull();
      expect(observed[observed.length - 1]).toBeNull();
      expect(client.getQueryData(key)).toBeNull();
    } finally {
      unsubscribe();
      client.clear();
    }
  });

  it("cancels an older entity read before publishing committed deletion", async () => {
    const client = new QueryClient();
    const key = queryKeys.block("deleted-block");
    client.setQueryData(key, { uuid: "old-snapshot" });
    let finishRead: (value: { uuid: string }) => void = () => {
      throw new Error("entity read did not start");
    };
    const observer = new QueryObserver<{ uuid: string } | null>(client, {
      queryKey: key,
      queryFn: () =>
        new Promise((resolve) => {
          finishRead = resolve;
        }),
      staleTime: Number.POSITIVE_INFINITY,
    });
    const unsubscribe = observer.subscribe(() => undefined);
    try {
      const reading = observer.refetch();
      await applyDomainEvent(
        client,
        committed({
          blocks: [{ uuid: "deleted-block", before: placement("page-a"), after: null }],
        }),
      );
      finishRead({ uuid: "stale-read-result" });
      await reading;

      expect(observer.getCurrentResult().data).toBeNull();
      expect(client.getQueryData(key)).toBeNull();
    } finally {
      unsubscribe();
      client.clear();
    }
  });

  it.each([
    ["page", queryKeys.page("page-a")],
    ["document", queryKeys.pageDocument("page-a")],
    ["render snapshot", queryKeys.pageRender("page-a")],
    ["block", queryKeys.block("block-a")],
  ])(
    "reopens a recreated %s without publishing a cached deletion after unmount",
    async (_name, key) => {
      const client = new QueryClient();
      client.setQueryData(key, { uuid: "page-a" });
      let persisted: { uuid: string } | null = null;
      const query = {
        queryKey: key,
        queryFn: async () => persisted,
        staleTime: Number.POSITIVE_INFINITY,
      };
      const mounted = new QueryObserver(client, query);
      const unsubscribe = mounted.subscribe(() => undefined);
      await applyDomainEvent(
        client,
        committed({
          deletedPages: ["page-a"],
          blocks: [{ uuid: "block-a", before: placement("page-a"), after: null }],
        }),
      );
      expect(mounted.getCurrentResult().data).toBeNull();
      unsubscribe();

      persisted = { uuid: "page-a" };
      await applyDomainEvent(
        client,
        committed({
          changedPages: ["page-a"],
          blocks: [{ uuid: "block-a", before: null, after: placement("page-a") }],
        }),
      );
      const reopened = new QueryObserver(client, query);
      const observed: unknown[] = [];
      const unsubscribeReopened = reopened.subscribe((result) => {
        if (result.isSuccess) observed.push(result.data);
      });
      try {
        expect(reopened.getCurrentResult().isPending).toBe(true);
        await reopened.refetch();
        expect(reopened.getCurrentResult().data).toEqual(persisted);
        expect(observed).not.toContain(null);
      } finally {
        unsubscribeReopened();
        client.clear();
      }
    },
  );

  it("refreshes only history when a committed stale history entry is skipped", async () => {
    const client = new QueryClient();
    client.setQueryData(queryKeys.history, {});
    const unchanged = [
      queryKeys.pageList("notes", 10_000),
      queryKeys.pageDocument("page-a"),
      queryKeys.pageRender("page-a"),
      queryKeys.children("page-a"),
      queryKeys.graph(null),
      queryKeys.settings,
    ];
    for (const key of unchanged) client.setQueryData(key, {});
    client.setQueryData(queryKeys.searchRevision, 4);

    await applyDomainEvent(client, committed({ historyChanged: true }));

    expect(client.getQueryState(queryKeys.history)?.isInvalidated).toBe(true);
    for (const key of unchanged) expect(client.getQueryState(key)?.isInvalidated).toBe(false);
    expect(client.getQueryData(queryKeys.searchRevision)).toBe(4);
  });

  it("invalidates a changed page and title-derived views", async () => {
    const client = new QueryClient();
    client.setQueryData(queryKeys.pages, []);
    client.setQueryData(queryKeys.journals, []);
    client.setQueryData(queryKeys.journal("2026-07-17"), { uuid: "page-a" });
    client.setQueryData(queryKeys.page("page-a"), { uuid: "page-a" });
    client.setQueryData(queryKeys.graph(null), { items: [], edges: [] });
    client.setQueryData(queryKeys.handwritingStatus("page-a"), { heads: [] });

    await applyDomainEvent(client, {
      kind: "pages_changed",
      page_uuids: ["page-a"],
    });

    expect(client.getQueryState(queryKeys.pages)?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.journals)?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.journal("2026-07-17"))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.page("page-a"))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.graph(null))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.handwritingStatus("page-a"))?.isInvalidated).toBe(true);
  });

  it("invalidates every handwriting status when publication progresses", async () => {
    const client = new QueryClient();
    client.setQueryData(queryKeys.handwritingStatus("page-a"), { heads: [] });
    client.setQueryData(queryKeys.syncStatus, { state: "idle" });

    await applyDomainEvent(client, { kind: "sync_status_changed" });

    expect(client.getQueryState(queryKeys.syncStatus)?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.handwritingStatus("page-a"))?.isInvalidated).toBe(true);
  });

  it("invalidates a changed block and its containing child list", async () => {
    const client = new QueryClient();
    client.setQueryData(queryKeys.block("block-a"), { uuid: "block-a" });
    client.setQueryData(queryKeys.block("block-b"), { uuid: "block-b" });
    client.setQueryData(queryKeys.children("parent-a"), []);
    client.setQueryData(queryKeys.pageDocument("page-a"), { revision: "d1" });
    client.setQueryData(queryKeys.pageRender("page-a"), { document: { revision: "d1" } });
    client.setQueryData(queryKeys.pages, []);
    client.setQueryData(queryKeys.graph(null), { items: [], edges: [] });

    await applyDomainEvent(client, {
      kind: "blocks_changed",
      block_uuids: ["block-a"],
      container_uuids: ["parent-a"],
    });

    expect(client.getQueryState(queryKeys.block("block-a"))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.block("block-b"))?.isInvalidated).toBe(false);
    expect(client.getQueryState(queryKeys.children("parent-a"))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.pageDocument("page-a"))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.pageRender("page-a"))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.pages)?.isInvalidated).toBe(false);
    expect(client.getQueryState(queryKeys.graph(null))?.isInvalidated).toBe(false);
  });

  it("removes deleted block snapshots", async () => {
    const client = new QueryClient();
    client.setQueryData(queryKeys.block("deleted"), { uuid: "deleted" });

    await applyDomainEvent(client, {
      kind: "blocks_deleted",
      block_uuids: ["deleted"],
      container_uuids: [],
    });

    expect(client.getQueryData(queryKeys.block("deleted"))).toBeUndefined();
  });

  it("invalidates the complete backend cache after a workspace replacement", async () => {
    const client = new QueryClient();
    client.setQueryData(queryKeys.settings, { windowDecorationMode: "native" });
    client.setQueryData(queryKeys.history, [1, 0]);

    await applyDomainEvent(client, { kind: "workspace_changed" });

    expect(client.getQueryState(queryKeys.settings)?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.history)?.isInvalidated).toBe(true);
  });

  it("invalidates server AI state after a remote setting mutation", async () => {
    const client = new QueryClient();
    client.setQueryData(queryKeys.serverAi, { generationState: "active" });

    await applyDomainEvent(client, { kind: "server_ai_changed" });

    expect(client.getQueryState(queryKeys.serverAi)?.isInvalidated).toBe(true);
  });

  it("invalidates structure without refetching graph data", async () => {
    const client = new QueryClient();
    client.setQueryData(queryKeys.children("parent-a"), []);
    client.setQueryData(queryKeys.children("parent-b"), []);
    client.setQueryData(queryKeys.graph(null), { items: [], edges: [] });
    client.setQueryData(queryKeys.pageRender("page-a"), { document: { revision: "d1" } });

    await applyDomainEvent(client, {
      kind: "structure_changed",
      block_uuids: ["moved"],
    });

    expect(client.getQueryState(queryKeys.children("parent-a"))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.children("parent-b"))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.pageRender("page-a"))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.graph(null))?.isInvalidated).toBe(false);
  });

  it("invalidates only attachments for affected owners", async () => {
    const client = new QueryClient();
    client.setQueryData(queryKeys.attachments("parent-a"), []);
    client.setQueryData(queryKeys.attachments("parent-b"), []);
    client.setQueryData(queryKeys.attachmentImages(["attachment-a"]), []);
    client.setQueryData(queryKeys.pageRender("page-a"), { document: { revision: "d1" } });

    await applyDomainEvent(client, {
      kind: "attachments_changed",
      owner_uuids: ["parent-a"],
    });

    expect(client.getQueryState(queryKeys.attachments("parent-a"))?.isInvalidated).toBe(true);
    expect(client.getQueryState(queryKeys.attachments("parent-b"))?.isInvalidated).toBe(false);
    expect(client.getQueryState(queryKeys.attachmentImages(["attachment-a"]))?.isInvalidated).toBe(
      true,
    );
    expect(client.getQueryState(queryKeys.pageRender("page-a"))?.isInvalidated).toBe(true);
  });
});

function committed(changes: Partial<ChangeSet>): DomainEvent {
  return {
    kind: "mutation_committed",
    commit: {
      commitVersion: 1,
      changes: {
        changedPages: [],
        deletedPages: [],
        affectedPages: [],
        blocks: [],
        graphChangedContent: [],
        attachmentOwners: [],
        historyChanged: false,
        ...changes,
      },
    },
  };
}

function placement(pageUuid: string, parentUuid: string | null = null) {
  return { pageUuid, parentUuid, orderKey: "a0" };
}
