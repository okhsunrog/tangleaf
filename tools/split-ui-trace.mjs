await (async () => {
  window.__splitUiTrace?.dispose();
  if (window.__tangleafReleaseProbe?.result().status !== "armed")
    throw Error("Arm the unchanged release probe first");
  const source = await window.__TAURI__.core.invoke("get_block", {
    uuid: window.__tangleafReleaseProbe.result().sourceUuid,
  });
  const expectedLastText = source.markdown
    .split(/\n[ \t]*(?:\n[ \t]*)+/)
    .map((p) => p.trim())
    .filter(Boolean)
    .at(-1);
  const entries = [],
    restorers = [];
  let active = false,
    clickAt = null,
    rafOrdinal = 0,
    timer = null;
  const counts = {};
  const now = () => performance.now();
  const push = (e) => {
    if (!active) return;
    counts[e.name] = (counts[e.name] ?? 0) + 1;
    if (entries.length < 20000) entries.push({ ...e, observedAt: now() });
  };
  window.__tangleafUiProfile = push;
  function wrap(object, name, label, detail) {
    const original = object[name];
    if (typeof original !== "function") return;
    const own = Object.prototype.hasOwnProperty.call(object, name);
    const wrapped = function (...args) {
      if (!active) return original.apply(this, args);
      const start = now();
      try {
        return original.apply(this, args);
      } finally {
        push({ name: label, start, end: now(), detail: detail?.(this, args) });
      }
    };
    object[name] = wrapped;
    restorers.push(() => {
      if (object[name] !== wrapped) return;
      if (own) object[name] = original;
      else delete object[name];
    });
  }
  wrap(Element.prototype, "getBoundingClientRect", "dom.rect", (el) => ({
    tag: el.tagName,
    cm: el.classList?.contains("cm-content") ?? false,
  }));
  wrap(Element.prototype, "getClientRects", "dom.rects");
  wrap(Range.prototype, "getBoundingClientRect", "range.rect");
  wrap(Range.prototype, "getClientRects", "range.rects");
  wrap(HTMLElement.prototype, "focus", "dom.focus", (el) => ({
    tag: el.tagName,
    cm: el.classList?.contains("cm-content") ?? false,
  }));
  wrap(Element.prototype, "scrollIntoView", "dom.scrollIntoView");
  wrap(Element.prototype, "scrollTo", "dom.scrollTo", (_element, args) => ({
    top: typeof args[0] === "object" ? args[0]?.top : args[1],
    behavior: typeof args[0] === "object" ? args[0]?.behavior : undefined,
  }));
  const raf = window.requestAnimationFrame;
  const wrappedRaf = function (callback) {
    const id = ++rafOrdinal,
      requestedAt = now();
    return raf.call(this, function (timestamp) {
      const start = now();
      try {
        return callback(timestamp);
      } finally {
        push({ name: "raf.callback", start, end: now(), detail: { id, requestedAt, timestamp } });
      }
    });
  };
  window.requestAnimationFrame = wrappedRaf;
  restorers.push(() => {
    if (window.requestAnimationFrame === wrappedRaf) window.requestAnimationFrame = raf;
  });
  const firsts = {};
  const eligibilityChanges = [];
  let previousEligibility = false;
  function observe(reason) {
    if (!active) return;
    const release = window.__tangleafReleaseProbe.result();
    let eligible = false;
    let state = { row: false, editor: false };
    if (release.targetUuid) {
      const row = document.querySelector(`[data-block-uuid="${CSS.escape(release.targetUuid)}"]`);
      const editor = row?.querySelector(".cm-content[contenteditable=true]");
      state = { row: Boolean(row), editor: Boolean(editor) };
      if (row && firsts.row === undefined) firsts.row = now();
      if (editor) {
        if (firsts.editor === undefined) firsts.editor = now();
        const focused = editor.contains(document.activeElement);
        if (focused && firsts.focused === undefined) firsts.focused = now();
        const rect = editor.getBoundingClientRect();
        const visible =
          rect.width > 0 && rect.height > 0 && rect.bottom > 0 && rect.top < innerHeight;
        if (visible && firsts.visible === undefined) firsts.visible = now();
        const exact =
          [...editor.querySelectorAll(".cm-line")].map((e) => e.textContent).join("\n") ===
          expectedLastText;
        if (exact && firsts.exact === undefined) firsts.exact = now();
        eligible = exact && focused && visible;
        state = {
          ...state,
          focused,
          visible,
          exact,
          top: rect.top,
          bottom: rect.bottom,
          scrollTop: row.closest("[data-workspace-scroll]")?.scrollTop,
        };
        if (eligible && firsts.eligible === undefined) {
          firsts.eligible = now();
          firsts.eligibleReason = reason;
        }
      }
    }
    if (eligible !== previousEligibility) {
      eligibilityChanges.push({ at: now() - clickAt, eligible, reason, state });
      previousEligibility = eligible;
    }
    if (release.status === "ready") {
      firsts.probeReadyObserved = now();
      active = false;
      clearTimeout(timer);
    }
  }
  const mutation = new MutationObserver(() => observe("mutation"));
  mutation.observe(document.body, { childList: true, subtree: true, characterData: true });
  const focus = () => observe("focus");
  document.addEventListener("focusin", focus, true);
  const click = (event) => {
    const button = event.target.closest?.("button");
    if (
      !event.isTrusted ||
      !button?.closest("[data-sonner-toast]") ||
      button.textContent.trim() !== "Split"
    )
      return;
    entries.length = 0;
    eligibilityChanges.length = 0;
    previousEligibility = false;
    for (const k of Object.keys(firsts)) delete firsts[k];
    for (const k of Object.keys(counts)) delete counts[k];
    clickAt = now();
    active = true;
    timer = setTimeout(() => {
      active = false;
    }, 5000);
    const poll = () => {
      observe("raf");
      if (active) raf.call(window, poll);
    };
    raf.call(window, poll);
  };
  document.addEventListener("click", click, true);
  window.__splitUiTrace = {
    result() {
      return {
        diagnosticOnly: true,
        schemaVersion: 2,
        clickAt,
        timeOrigin: performance.timeOrigin,
        active,
        firsts: Object.fromEntries(
          Object.entries(firsts).map(([k, v]) => [k, typeof v === "number" ? v - clickAt : v]),
        ),
        counts: { ...counts },
        eligibilityChanges: eligibilityChanges.map((entry) => ({ ...entry })),
        entries: entries.map((e) => ({
          ...e,
          start: e.start - clickAt,
          end: e.end - clickAt,
          observedAt: e.observedAt - clickAt,
          detail:
            e.detail?.requestedAt !== undefined
              ? {
                  ...e.detail,
                  requestedAt: e.detail.requestedAt - clickAt,
                  timestamp: e.detail.timestamp - clickAt,
                }
              : e.detail,
        })),
        release: window.__tangleafReleaseProbe.result(),
        limit: 20000,
      };
    },
    dispose() {
      active = false;
      clearTimeout(timer);
      mutation.disconnect();
      document.removeEventListener("focusin", focus, true);
      document.removeEventListener("click", click, true);
      for (const restore of restorers.reverse()) restore();
      delete window.__tangleafUiProfile;
      delete window.__splitUiTrace;
    },
  };
  return { installed: true, diagnosticOnly: true };
})();
