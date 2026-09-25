// @vitest-environment jsdom
import { act } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vite-plus/test";
import { PageSessionProvider } from "@/features/pages/page-session";
import {
  PaneContentKind,
  currentDisposition,
  homeTarget,
  pageTarget,
  type PaneId,
} from "@/features/workspace/workspace-model";
import { mayLeavePane, resetPaneLeaveGuards } from "@/features/workspace/pane-leave-guard";
import { useWorkspaceStore } from "@/features/workspace/workspace-store";

const paneId = () => useWorkspaceStore.getState().primaryPaneId as PaneId;
import type { InkHistorySnapshot, InkVersionInfo, Page } from "@/lib/bindings";
import { HandwritingNoteView } from "./handwriting-note-view";
import { getWriter, resetHandwritingSessions } from "./handwriting-session";
import { useHandwritingPreference } from "./input-capabilities";
import { emptyDraft } from "./ink-model";

const api = vi.hoisted(() => ({
  loadHandwritingNote: vi.fn(),
  saveHandwritingPatch: vi.fn(),
  completeHandwritingNote: vi.fn(),
  completeAllHandwriting: vi.fn(),
  setHandwritingBackground: vi.fn(),
  handwritingHistory: vi.fn(),
  handwritingNoteStatus: vi.fn(),
  renamePage: vi.fn(),
  getPage: vi.fn(),
  notifyRetryableError: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, ...api };
});
vi.mock("@/lib/notify", () => ({
  notifyError: vi.fn(),
  notifyInfo: vi.fn(),
  notifySuccess: vi.fn(),
  notifyRetryableError: api.notifyRetryableError,
}));

const page: Page = {
  uuid: "019f0000-0000-7000-8000-00000000000a",
  kind: { kind: "handwriting" },
  title: "Sketch",
  layout: "outline",
  titleRevision: "title-revision",
  createdAt: 0,
  updatedAt: 0,
};

const history = (revision: string | null): InkHistorySnapshot => ({
  snapshot: { draft: emptyDraft(), revision },
  canUndo: false,
  canRedo: false,
});

let container: HTMLDivElement;
let root: Root;
let queryClient: QueryClient;

beforeAll(() => {
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
});

beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  for (const mock of Object.values(api)) mock.mockReset();
  api.loadHandwritingNote.mockResolvedValue(history("revision-1"));
  api.completeHandwritingNote.mockResolvedValue({});
  api.setHandwritingBackground.mockResolvedValue(undefined);
  api.handwritingNoteStatus.mockResolvedValue({
    pageUuid: page.uuid,
    revision: "revision-1",
    unpublishedChanges: false,
    publicationRequested: false,
    baseVersion: null,
    heads: [],
  });
  queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  useWorkspaceStore.getState().dispatch({ type: "reset" });
  // Drawing without a detected pen is the explicit device preference.
  useHandwritingPreference.setState({ mouseEnabled: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  resetPaneLeaveGuards();
  await act(async () => root.unmount());
  container.remove();
  resetHandwritingSessions();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function paneContentKind() {
  const state = useWorkspaceStore.getState();
  return state.panes[state.primaryPaneId]?.content.kind;
}

async function mount(onDelete: (page: Page) => void = () => {}, onBack?: () => void) {
  const paneId = useWorkspaceStore.getState().primaryPaneId;
  useWorkspaceStore.getState().dispatch({
    type: "open_target",
    target: pageTarget(page.uuid),
    disposition: currentDisposition,
  });
  await act(async () =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <PageSessionProvider>
          <HandwritingNoteView
            paneId={paneId as PaneId}
            page={page}
            onSaved={() => {}}
            onDelete={onDelete}
            onBack={onBack}
          />
        </PageSessionProvider>
      </QueryClientProvider>,
    ),
  );
}

/** Menus and popovers are portalled out of the pane, so they are looked up in the document. */
function menuItem(label: string, role = "menuitem") {
  const found = [...document.querySelectorAll<HTMLElement>(`[role="${role}"]`)].find(
    (element) => element.textContent?.trim() === label,
  );
  if (!found) throw new Error(`no menu item labelled ${label}`);
  return found;
}

/** The grid is a checkbox in the note menu, not a toggle in the toolbar. */
async function toggleNoteMenu() {
  await act(async () => {
    button("Note options").click();
  });
}

function gridItem() {
  return menuItem("Grid paper", "menuitemcheckbox");
}

async function toggleGrid() {
  await toggleNoteMenu();
  await act(async () => {
    gridItem().click();
  });
}

/** Save errors, conflicts and the sheet limit live behind one status chip in the toolbar. */
function statusChip() {
  return container.querySelector<HTMLElement>("[data-ink-status]");
}

async function openStatus() {
  const chip = statusChip();
  if (!chip) throw new Error("no status chip");
  await act(async () => {
    chip.click();
  });
}

function statusDetails() {
  return document.querySelector<HTMLElement>("[data-ink-status-details]");
}

function statusAction(label: string) {
  const found = [
    ...document.querySelectorAll<HTMLElement>("[data-ink-status-details] button"),
  ].find((element) => element.textContent?.trim() === label);
  if (!found) throw new Error(`no status action labelled ${label}`);
  return found;
}

function head(versionUuid: string): InkVersionInfo {
  return {
    versionUuid,
    parents: [],
    rootHash: `hash-${versionUuid}`,
    deviceName: "Tablet",
    deviceId: `device-${versionUuid}`,
    modifiedAtMs: 0,
    available: true,
  };
}

function options(label: string) {
  const found = [...document.querySelectorAll<HTMLElement>("[data-ink-tool-options] button")].find(
    (element) =>
      element.textContent?.trim() === label || element.getAttribute("aria-label") === label,
  );
  if (!found) throw new Error(`no tool option labelled ${label}`);
  return found;
}

/** The rename dialog is portalled out of the pane, so it is looked up in the document. */
function renameField(): HTMLInputElement;
function renameField(required: false): HTMLInputElement | null;
function renameField(required = true) {
  const field = document.querySelector<HTMLInputElement>('input[aria-label="Note name"]');
  if (!field && required) throw new Error("no rename field");
  return field;
}

function button(label: string) {
  const found = [...container.querySelectorAll("button")].find(
    (element) =>
      element.textContent?.trim() === label || element.getAttribute("aria-label") === label,
  );
  if (!found) throw new Error(`no button labelled ${label}`);
  return found;
}

async function visibility(state: "hidden" | "visible") {
  Object.defineProperty(document, "visibilityState", { value: state, configurable: true });
  await act(async () => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
}

it("opens the note for editing and reports its background transitions", async () => {
  await mount();

  expect(api.loadHandwritingNote).toHaveBeenCalledWith(page.uuid, true);
  expect(container.querySelector("canvas")).not.toBeNull();

  await visibility("hidden");

  expect(api.setHandwritingBackground).toHaveBeenLastCalledWith(true);
  expect(api.completeHandwritingNote).toHaveBeenCalledWith(page.uuid);
});

it("re-adopts the note with a new revision when the window becomes visible", async () => {
  await mount();

  api.loadHandwritingNote.mockResolvedValue(history("revision-2"));
  let finishCompletion!: () => void;
  api.completeHandwritingNote.mockReturnValue(
    new Promise((resolve) => {
      finishCompletion = () => resolve({});
    }),
  );
  await visibility("hidden");
  await visibility("visible");

  expect(api.setHandwritingBackground).toHaveBeenLastCalledWith(false);
  // The re-read waits for the completion the suspend started.
  expect(api.loadHandwritingNote).toHaveBeenCalledTimes(1);

  await act(async () => finishCompletion());

  expect(api.loadHandwritingNote).toHaveBeenLastCalledWith(page.uuid, true);
  expect(api.loadHandwritingNote).toHaveBeenCalledTimes(2);
});

it("stays on the note when the flush before Back fails", async () => {
  api.saveHandwritingPatch.mockRejectedValue(new Error("disk full"));
  await mount();

  await toggleGrid();
  let allowed = true;
  await act(async () => {
    allowed = await mayLeavePane(paneId());
  });

  expect(allowed).toBe(false);
  expect(paneContentKind()).toBe(PaneContentKind.Page);
  expect(api.completeHandwritingNote).not.toHaveBeenCalled();
  expect(statusChip()?.textContent).toContain("Not saved");
  await openStatus();
  expect(statusDetails()?.textContent).toContain("Could not save your changes");
});

it("leaves after a successful flush and offers a retry when completion fails", async () => {
  api.saveHandwritingPatch.mockResolvedValue("revision-2");
  api.completeHandwritingNote.mockRejectedValue(new Error("outbox unavailable"));
  await mount();

  await toggleGrid();
  let allowed = false;
  await act(async () => {
    allowed = await mayLeavePane(paneId());
  });
  expect(allowed).toBe(true);
  // The pane frame navigates once the guard agrees; the editor no longer does it itself.
  await act(async () => {
    useWorkspaceStore.getState().dispatch({
      type: "open_target",
      target: homeTarget,
      disposition: currentDisposition,
    });
  });

  expect(paneContentKind()).toBe(PaneContentKind.Home);
  expect(api.saveHandwritingPatch).toHaveBeenCalledWith(
    page.uuid,
    expect.objectContaining({ background: "grid" }),
    "revision-1",
  );
  expect(api.notifyRetryableError).toHaveBeenCalledWith(
    "Could not prepare sync",
    expect.any(Error),
    expect.any(Function),
  );

  api.completeHandwritingNote.mockResolvedValue({});
  const retry = api.notifyRetryableError.mock.calls[0]![2] as () => void;
  await act(async () => retry());

  expect(api.completeHandwritingNote).toHaveBeenCalledTimes(2);
});

it("opens read-only without a pen and without the mouse preference", async () => {
  useHandwritingPreference.setState({ mouseEnabled: false });
  await mount();

  expect(api.loadHandwritingNote).toHaveBeenCalledWith(page.uuid, false);
  // The row stays — Back and the note menu still work — but it offers no tools.
  expect(container.querySelectorAll('[role="toolbar"] button[aria-pressed]')).toHaveLength(0);
  expect(container.textContent).toContain("Connect a pen or enable mouse drawing in Settings");
  expect(container.querySelector("canvas")).not.toBeNull();
});

it("retries the gestures an earlier mount could not store before reading again", async () => {
  api.saveHandwritingPatch.mockRejectedValue(new Error("disk full"));
  await mount();

  await toggleGrid();
  await openStatus();
  expect(statusDetails()?.textContent).toContain("Could not save your changes");

  await act(async () => root.unmount());
  root = createRoot(container);
  api.loadHandwritingNote.mockClear();
  api.saveHandwritingPatch.mockClear();

  await mount();

  // The unsaved gesture is on the canvas again and no snapshot replaced it.
  await toggleNoteMenu();
  expect(gridItem().getAttribute("aria-checked")).toBe("true");
  await toggleNoteMenu();
  expect(api.loadHandwritingNote).not.toHaveBeenCalled();
  await openStatus();
  expect(statusDetails()?.textContent).toContain("Could not save your changes");

  api.saveHandwritingPatch.mockResolvedValue("revision-2");
  await act(async () => {
    statusAction("Retry saving").click();
  });

  expect(api.saveHandwritingPatch).toHaveBeenLastCalledWith(
    page.uuid,
    expect.objectContaining({ background: "grid" }),
    "revision-1",
  );
  expect(api.loadHandwritingNote).toHaveBeenCalledWith(page.uuid, true);
});

it("stores queued gestures before deleting and never asks to retry sync", async () => {
  const { CommandFailure } = await import("@/lib/api");
  api.saveHandwritingPatch.mockResolvedValue("revision-2");
  api.completeHandwritingNote.mockRejectedValue(
    new CommandFailure({ code: "not_found", message: "page is gone" }),
  );
  const deleted = vi.fn();
  await mount(deleted);

  await toggleGrid();
  await toggleNoteMenu();
  await act(async () => {
    menuItem("Delete note").click();
  });

  expect(api.saveHandwritingPatch).toHaveBeenCalledTimes(1);
  expect(deleted).toHaveBeenCalledWith(page);

  await act(async () => root.unmount());
  root = createRoot(container);

  expect(api.notifyRetryableError).not.toHaveBeenCalled();
});

it("re-reads the note for editing once a pen or the mouse preference appears", async () => {
  useHandwritingPreference.setState({ mouseEnabled: false });
  await mount();

  expect(api.loadHandwritingNote).toHaveBeenCalledWith(page.uuid, false);
  expect(container.querySelectorAll('[role="toolbar"] button[aria-pressed]')).toHaveLength(0);

  api.saveHandwritingPatch.mockResolvedValue("revision-2");
  await act(async () => {
    useHandwritingPreference.setState({ mouseEnabled: true });
  });

  // A read-only open holds no session, so the note has to be read again before
  // anything drawn on it can be stored.
  expect(api.loadHandwritingNote).toHaveBeenLastCalledWith(page.uuid, true);
  await toggleGrid();
  expect(api.saveHandwritingPatch).toHaveBeenCalledWith(
    page.uuid,
    expect.objectContaining({ background: "grid" }),
    "revision-1",
  );
});

it("closes the session of a note that was deleted while gestures were queued", async () => {
  const { CommandFailure } = await import("@/lib/api");
  api.saveHandwritingPatch.mockRejectedValue(
    new CommandFailure({ code: "not_found", message: "page is gone" }),
  );
  await mount();

  await toggleGrid();
  expect(getWriter(page.uuid)).not.toBeNull();

  await act(async () => root.unmount());
  root = createRoot(container);

  // Keeping the writer would strand it for the life of the process: its
  // gestures can never be stored against a note that no longer exists.
  expect(getWriter(page.uuid)).toBeNull();
  expect(api.completeHandwritingNote).not.toHaveBeenCalled();
});

it("renames from a dialog instead of a field on the sheet", async () => {
  api.renamePage.mockResolvedValue({ ...page, title: "Meeting", titleRevision: "title-2" });
  await mount();

  // Nothing on the drawing screen takes the caret; the keyboard cannot meet an armed pen.
  expect(container.querySelector("textarea")).toBeNull();
  expect(container.querySelector("input")).toBeNull();

  await act(async () => {
    button("Rename note").click();
  });
  const field = renameField();
  await act(async () => {
    // React tracks the last value it wrote; the native setter is what a real keystroke reaches.
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(
      field,
      "Meeting",
    );
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    field.form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });

  expect(api.renamePage).toHaveBeenCalledWith(page.uuid, "Meeting", "title-revision");
  expect(renameField(false)).toBeNull();
});

it("leaves undo to the rename field while the caret is in it", async () => {
  api.saveHandwritingPatch.mockResolvedValue("revision-2");
  api.handwritingHistory.mockResolvedValue({
    baseRevision: "revision-2",
    revision: "revision-3",
    canUndo: false,
    canRedo: true,
    patch: { order: [], upserts: [], background: "plain" },
  });
  await mount();

  await toggleGrid();

  await act(async () => {
    button("Rename note").click();
  });
  const title = renameField();
  await act(async () => {
    title.dispatchEvent(
      new KeyboardEvent("keydown", { key: "z", ctrlKey: true, bubbles: true, cancelable: true }),
    );
  });
  expect(api.handwritingHistory).not.toHaveBeenCalled();

  await act(async () => {
    button("Undo stroke").click();
  });
  expect(api.handwritingHistory).toHaveBeenCalledWith(page.uuid, false, "revision-2");
});

it("keeps one toolbar row and moves the tool options into a popover", async () => {
  await mount();

  // Every strip of chrome is sheet the pen cannot use; there is exactly one.
  expect(container.querySelectorAll('[role="toolbar"]')).toHaveLength(1);
  expect(document.querySelector("[data-ink-tool-options]")).toBeNull();

  // The first tap on a tool switches to it; only the second opens its options.
  await act(async () => {
    button("Eraser").click();
  });
  expect(document.querySelector("[data-ink-tool-options]")).toBeNull();
  await act(async () => {
    button("Eraser options").click();
  });
  expect(document.querySelector("[data-ink-tool-options]")).not.toBeNull();

  await act(async () => {
    options("Pixel eraser").click();
  });
  expect(options.bind(null, "Pixel eraser")).toThrow();
  expect(container.querySelectorAll('[role="toolbar"]')).toHaveLength(1);
});

it("offers Back in the toolbar only when the note has the screen", async () => {
  await mount();
  expect(button.bind(null, "Go back")).toThrow();

  await act(async () => root.unmount());
  root = createRoot(container);
  const back = vi.fn();
  await mount(undefined, back);

  await act(async () => {
    button("Go back").click();
  });
  expect(back).toHaveBeenCalledTimes(1);
});

it("keeps the grid in the note menu, where it is not a second selected tool", async () => {
  api.saveHandwritingPatch.mockResolvedValue("revision-2");
  await mount();

  expect(button.bind(null, "Grid paper")).toThrow();
  await toggleNoteMenu();
  expect(gridItem().getAttribute("aria-checked")).toBe("false");
  await act(async () => {
    gridItem().click();
  });

  expect(api.saveHandwritingPatch).toHaveBeenCalledWith(
    page.uuid,
    expect.objectContaining({ background: "grid" }),
    "revision-1",
  );
  // Only the tool in use is pressed in the bar.
  const pressed = container.querySelectorAll('[role="toolbar"] button[aria-pressed="true"]');
  expect([...pressed].map((element) => element.getAttribute("aria-label"))).toEqual([
    "Pen, medium",
  ]);
});

it("shows the most urgent status on the chip and every status behind it", async () => {
  api.saveHandwritingPatch.mockRejectedValue(new Error("disk full"));
  api.handwritingNoteStatus.mockResolvedValue({
    pageUuid: page.uuid,
    revision: "revision-1",
    unpublishedChanges: true,
    publicationRequested: false,
    baseVersion: null,
    heads: [head("a"), head("b")],
  });
  await mount();

  expect(statusChip()?.textContent).toBe("2 versions");
  expect(container.textContent).not.toContain("Unsent");

  await toggleGrid();

  // A failing save outranks the conflict; the chip counts what else is waiting.
  expect(statusChip()?.textContent).toBe("Not saved+1");
  await openStatus();
  const details = statusDetails()?.textContent ?? "";
  expect(details).toContain("Could not save your changes. disk full");
  expect(details).toContain("This note has 2 versions from other devices");
  expect(details.indexOf("Could not save")).toBeLessThan(details.indexOf("versions"));
  // Nothing about the note is a row above or below the sheet any more.
  expect(container.querySelector("footer")).toBeNull();
});

it("says Unsent on its own when there is nothing else to report", async () => {
  api.handwritingNoteStatus.mockResolvedValue({
    pageUuid: page.uuid,
    revision: "revision-1",
    unpublishedChanges: true,
    publicationRequested: false,
    baseVersion: null,
    heads: [],
  });
  await mount();

  expect(statusChip()).toBeNull();
  expect(container.querySelector('[role="toolbar"]')?.textContent).toContain("Unsent");
});

it("picks the pen and its width with one tap on a preset", async () => {
  await mount();
  const pressed = (label: string) => button(label).getAttribute("aria-pressed");

  expect(pressed("Pen, medium")).toBe("true");
  await act(async () => {
    button("Eraser").click();
  });
  expect(pressed("Pen, medium")).toBe("false");

  await act(async () => {
    button("Pen, bold").click();
  });
  expect(pressed("Pen, bold")).toBe("true");
  expect(pressed("Eraser")).toBe("false");
  expect(pressed("Pen, medium")).toBe("false");
  // A preset is the whole choice; there are no pen options to open.
  await act(async () => {
    button("Pen, bold").click();
  });
  expect(document.querySelector("[data-ink-tool-options]")).toBeNull();
});

it("folds the presets into one pen with a width popover on a narrow bar", async () => {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    width: 420,
    height: 44,
  } as DOMRect);
  await mount();

  expect(button.bind(null, "Pen, fine")).toThrow();
  await act(async () => {
    button("Pen options").click();
  });
  await act(async () => {
    options("Pen width 5").click();
  });
  expect(options.bind(null, "Pen width 5")).toThrow();

  await act(async () => {
    button("Pen options").click();
  });
  expect(options("Pen width 5").getAttribute("aria-pressed")).toBe("true");
});

it("holds the tools during a stroke without dimming them", async () => {
  await mount();
  const canvas = container.querySelector("canvas");
  if (!canvas) throw new Error("no canvas");
  canvas.setPointerCapture = vi.fn();
  canvas.releasePointerCapture = vi.fn();
  canvas.hasPointerCapture = () => true;
  vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
    left: 0,
    top: 0,
    right: 500,
    bottom: 700,
    width: 500,
    height: 700,
  } as DOMRect);
  const down = new Event("pointerdown", { bubbles: true, cancelable: true });
  Object.assign(down, {
    pointerId: 1,
    pointerType: "pen",
    clientX: 20,
    clientY: 30,
    pressure: 0.5,
    buttons: 1,
    button: 0,
  });
  await act(async () => {
    canvas.dispatchEvent(down);
  });

  // Nothing may switch under the gesture, but a bar that dimmed on every stroke would repaint
  // twice per stroke on e-ink.
  const eraser = button("Eraser");
  expect(eraser.disabled).toBe(true);
  expect(eraser.className).toContain("disabled:opacity-100");
  // Undo was already off before the stroke and keeps its dimmed look.
  const undo = button("Undo stroke");
  expect(undo.disabled).toBe(true);
  expect(undo.className).not.toContain("disabled:opacity-100");
});

it("names an icon-only status chip in words", async () => {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    width: 360,
    height: 44,
  } as DOMRect);
  api.handwritingNoteStatus.mockResolvedValue({
    pageUuid: page.uuid,
    revision: "revision-1",
    unpublishedChanges: false,
    publicationRequested: false,
    baseVersion: null,
    heads: [head("a"), head("b")],
  });
  await mount();

  expect(statusChip()?.textContent).toBe("");
  expect(statusChip()?.getAttribute("aria-label")).toBe("2 versions");
});
