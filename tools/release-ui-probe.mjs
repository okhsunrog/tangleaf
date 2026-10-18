await (async () => {
  // Inject this complete expression through Tauri MCP webview-execute-js.
  // Leading await makes the bridge await installation and report errors.
  // It observes a real Split click; it never invokes a mutation itself.
  await window.__tangleafReleaseProbe?.dispose();
  const internals = window.__TAURI_INTERNALS__;
  const tauri = window.__TAURI__;
  if (!internals?.invoke || !tauri?.event?.listen) throw Error("Inspection bridge is unavailable");
  const invoke = internals.invoke;
  const read = (command, args) => invoke.call(internals, command, args);
  const fetch = window.fetch;
  const callbacks = internals.callbacks;
  if (!(callbacks instanceof Map)) throw Error("Unsupported Tauri callback registry");
  const restorers = new Set();
  let transportReads = 0;
  let sample = null;
  const events = [];
  const readCommands = new Set([
    "get_block",
    "get_blocks",
    "get_contents",
    "get_page_document",
    "get_page_render_snapshot",
    "list_block_children",
  ]);
  const observer = new MutationObserver(() => schedule());
  observer.observe(document.body, { childList: true, subtree: true });

  function output() {
    if (!sample) return { status: "unarmed" };
    const time = (value) => (value === null || sample.click === null ? null : value - sample.click);
    return {
      status: sample.status,
      parts: sample.parts.length,
      pageUuid: sample.source.pageUuid,
      sourceUuid: sample.source.uuid,
      targetUuid: sample.target,
      clickTrusted: sample.click !== null,
      ipcStartMs: time(sample.ipcStart),
      ipcAckMs: time(sample.ack),
      mutationEventMs: time(sample.commit?.at ?? null),
      commitVersion: sample.commit?.version ?? null,
      workspaceEpoch: sample.commit?.epoch ?? null,
      editorReadyMs: time(sample.ready),
      mountedRowsAtReady: sample.mounted,
      ipc: sample.ipc.map(({ command, start, end }) => ({
        command,
        startMs: time(start),
        endMs: time(end),
      })),
    };
  }

  function correlate() {
    if (!sample?.changed) return;
    sample.commit =
      events.find(
        (event) =>
          event.at >= sample.click && sample.changed.every((block) => event.blocks.has(block.uuid)),
      ) ?? null;
  }

  function readyEditor() {
    if (!sample?.target || !sample.commit) return null;
    const row = document.querySelector(`[data-block-uuid="${CSS.escape(sample.target)}"]`);
    const editor = row?.querySelector(".cm-content[contenteditable=true]");
    if (!editor?.contains(document.activeElement)) return null;
    const rect = editor.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0 || rect.bottom <= 0 || rect.top >= innerHeight)
      return null;
    const text = Array.from(editor.querySelectorAll(".cm-line"), (line) => line.textContent).join(
      "\n",
    );
    return text === sample.parts.at(-1) ? editor : null;
  }

  function schedule() {
    if (!sample || sample.status !== "pending" || sample.framePending) return;
    const current = sample;
    current.framePending = true;
    requestAnimationFrame(() => {
      if (sample !== current || current.status !== "pending") return;
      current.framePending = false;
      const editor = readyEditor();
      if (!editor) return;
      requestAnimationFrame(() => {
        if (sample !== current || current.status !== "pending" || readyEditor() !== editor) return;
        requestAnimationFrame(() => {
          if (sample !== current || current.status !== "pending" || readyEditor() !== editor)
            return;
          current.ready = performance.now();
          current.mounted = document.querySelectorAll("[data-block-uuid]").length;
          current.status = "ready";
          clearTimeout(current.timeout);
        });
      });
    });
  }

  function click(event) {
    const button = event.target.closest?.("button");
    if (
      sample?.status !== "armed" ||
      !event.isTrusted ||
      !button?.closest("[data-sonner-toast]") ||
      button.textContent.trim() !== "Split"
    )
      return;
    sample.click = performance.now();
    sample.status = "pending";
    sample.timeout = setTimeout(() => {
      sample.status = "timeout";
    }, 15_000);
  }
  document.addEventListener("click", click, true);
  document.addEventListener("focusin", schedule, true);
  const unlisten = await tauri.event.listen("domain:event", ({ payload }) => {
    if (!sample || sample.status !== "pending" || payload.kind !== "mutation_committed") return;
    const { commit } = payload;
    events.push({
      at: performance.now(),
      version: commit.commitVersion,
      epoch: commit.workspaceEpoch,
      blocks: new Set(commit.changes.blocks.map((block) => block.uuid)),
    });
    correlate();
    schedule();
  });

  function observe(command, args, headers) {
    const current = sample;
    const split =
      current?.status === "pending" &&
      command === "split_block" &&
      args?.uuid === current.source.uuid &&
      args?.parts?.length === current.parts.length;
    const observed = current?.status === "pending" && (split || readCommands.has(command));
    const entry = observed ? { command, start: performance.now(), end: null } : null;
    if (!entry) return;
    current.ipc.push(entry);
    if (split) current.ipcStart = entry.start;
    const release = [];
    function hook(header, success) {
      const id = Number(headers.get(header));
      const original = callbacks.get(id);
      if (!original) throw Error("Missing Tauri callback");
      const restore = () => {
        if (callbacks.get(id) === wrapped) callbacks.set(id, original);
        restorers.delete(restore);
      };
      const wrapped = (result) => {
        for (const restore of release) restore();
        try {
          entry.end = performance.now();
          if (!success && split && sample === current) {
            current.status = "ipc_error";
            clearTimeout(current.timeout);
          } else if (success && split && sample === current) {
            current.ack = entry.end;
            current.changed = result;
            current.target = result.reduce(
              (last, block) => (!last || block.orderKey > last.orderKey ? block : last),
              null,
            )?.uuid;
            correlate();
            schedule();
          }
        } catch {
          current.status = "probe_error";
          clearTimeout(current.timeout);
        }
        return original(result);
      };
      callbacks.set(id, wrapped);
      restorers.add(restore);
      release.push(restore);
    }
    hook("Tauri-Callback", true);
    hook("Tauri-Error", false);
  }
  // Tauri defines invoke/ipc/postMessage as non-writable properties. Observe
  // their desktop fetch transport, preserving the real callback semantics.
  const wrappedFetch = function (input, options) {
    try {
      const url = new URL(typeof input === "string" ? input : input.url);
      if (
        (url.protocol === "ipc:" && url.hostname === "localhost") ||
        url.hostname === "ipc.localhost"
      ) {
        const command = decodeURIComponent(url.pathname.slice(1));
        if (command === "get_block") transportReads++;
        if (command === "split_block" || readCommands.has(command)) {
          observe(command, JSON.parse(options.body), new Headers(options.headers));
        }
      }
    } catch {
      if (sample?.status === "pending") {
        sample.status = "probe_error";
        clearTimeout(sample.timeout);
      }
    }
    return fetch.call(this, input, options);
  };
  window.fetch = wrappedFetch;

  const equivalent = (actual, expected) =>
    actual.length === expected.length &&
    actual.every((block, index) => {
      const other = expected[index];
      return (
        block.uuid === other.uuid &&
        block.markdown === other.markdown &&
        block.parentUuid === other.parentUuid &&
        block.orderKey === other.orderKey &&
        JSON.stringify(block.style) === JSON.stringify(other.style)
      );
    });
  async function verify(original) {
    if (!sample || (!original && !sample.changed)) throw Error("No captured split");
    const actual = await read("list_block_children", {
      pageUuid: sample.source.pageUuid,
      parentUuid: sample.source.parentUuid,
    });
    const expected = original
      ? sample.before
      : sample.before
          .flatMap((block) => (block.uuid === sample.source.uuid ? sample.changed : [block]))
          .sort((a, b) => (a.orderKey < b.orderKey ? -1 : a.orderKey > b.orderKey ? 1 : 0));
    const partsExact =
      original ||
      (sample.changed.length === sample.parts.length &&
        [...sample.changed]
          .sort((a, b) => (a.orderKey < b.orderKey ? -1 : 1))
          .every((block, index) => block.markdown === sample.parts[index]));
    return { exact: partsExact && equivalent(actual, expected), blocks: actual.length };
  }
  window.__tangleafReleaseProbe = {
    async arm(blockUuid, expectedParts) {
      if (sample?.status === "pending") throw Error("A sample is still running");
      const transportBefore = transportReads;
      const source = await read("get_block", { uuid: blockUuid });
      if (transportReads === transportBefore)
        throw Error("Desktop custom-protocol IPC is unavailable; restart this inspection instance");
      if (!source) throw Error("Source block not found");
      const parts = source.markdown
        .split(/\n[ \t]*(?:\n[ \t]*)+/)
        .map((p) => p.trim())
        .filter(Boolean);
      if (parts.length !== expectedParts) throw Error("Unexpected paragraph count");
      const before = await read("list_block_children", {
        pageUuid: source.pageUuid,
        parentUuid: source.parentUuid,
      });
      events.length = 0;
      sample = {
        status: "armed",
        source,
        parts,
        before,
        ipc: [],
        changed: null,
        commit: null,
        target: null,
        click: null,
        ipcStart: null,
        ack: null,
        ready: null,
        mounted: null,
        timeout: null,
        framePending: false,
      };
      return output();
    },
    result: output,
    verifySplit: () => verify(false),
    verifyOriginal: () => verify(true),
    async dispose() {
      clearTimeout(sample?.timeout);
      if (window.fetch === wrappedFetch) window.fetch = fetch;
      for (const restore of restorers) restore();
      observer.disconnect();
      document.removeEventListener("click", click, true);
      document.removeEventListener("focusin", schedule, true);
      await unlisten();
      sample = null;
      delete window.__tangleafReleaseProbe;
    },
  };
  return { installed: true };
})();
