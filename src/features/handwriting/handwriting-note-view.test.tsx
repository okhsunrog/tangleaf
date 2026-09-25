// @vitest-environment jsdom
import { act, createElement, type ComponentProps } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vite-plus/test";
import { ConfirmationProvider } from "@/app/confirmation";
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
import { resetEraserSettings, useEraserSettings } from "./eraser-settings";
import { HandwritingNoteView } from "./handwriting-note-view";
import { getWriter, resetHandwritingSessions } from "./handwriting-session";
import type { InkCanvas } from "./ink-canvas";
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

/** Every full panel refresh an overlay's close asks for, through the shared wrapper's decorator. */
const refresh = vi.hoisted(() => ({ soon: vi.fn() }));
vi.mock("@/app/eink-refresh", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/app/eink-refresh")>();
  return {
    ...actual,
    requestFullRefreshSoon: refresh.soon,
    refreshPanelAfterClose:
      <Rest extends unknown[]>(handler?: (open: boolean, ...rest: Rest) => void) =>
      (open: boolean, ...rest: Rest) => {
        if (!open) refresh.soon();
        handler?.(open, ...rest);
      },
  };
});

/** What the view hands the sheet, render by render. */
const canvasProps = vi.hoisted(() => vi.fn());
vi.mock("./ink-canvas", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./ink-canvas")>();
  return {
    ...actual,
    InkCanvas: (props: ComponentProps<typeof InkCanvas>) => {
      canvasProps(props);
      return createElement(actual.InkCanvas, props);
    },
  };
});
const lastCanvasProps = () =>
  canvasProps.mock.calls[canvasProps.mock.calls.length - 1]![0] as ComponentProps<typeof InkCanvas>;

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
  // The eraser's settings are per device and persisted; no test may inherit another's.
  resetEraserSettings();
  localStorage.clear();
  refresh.soon.mockReset();
  canvasProps.mockReset();
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
        <ConfirmationProvider>
          <PageSessionProvider>
            <HandwritingNoteView
              paneId={paneId as PaneId}
              page={page}
              onSaved={() => {}}
              onDelete={onDelete}
              onBack={onBack}
            />
          </PageSessionProvider>
        </ConfirmationProvider>
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

function eraserMenu() {
  return document.querySelector<HTMLElement>("[data-eraser-menu]");
}

/** The type the toolbar's eraser button shows. */
function toolbarEraserType() {
  return container
    .querySelector('[role="toolbar"] [data-eraser-type]')
    ?.getAttribute("data-eraser-type");
}

async function tap(element: HTMLElement) {
  await act(async () => {
    element.click();
  });
}

async function openEraserMenu() {
  await tap(button("Eraser"));
  await tap(button("Eraser options"));
}

function dialogButton(label: string) {
  const found = [
    ...document.querySelectorAll<HTMLElement>('[data-slot="dialog-content"] button'),
  ].find((element) => element.textContent?.trim() === label);
  if (!found) throw new Error(`no dialog button labelled ${label}`);
  return found;
}

const line = {
  id: "line",
  width: 2,
  points: [
    { x: 100, y: 100, pressure: 0.5, tiltX: 0, tiltY: 0, time: 0 },
    { x: 300, y: 100, pressure: 0.5, tiltX: 0, tiltY: 0, time: 1 },
  ],
};

const inked = (revision: string): InkHistorySnapshot => ({
  snapshot: { draft: { ...emptyDraft(), strokes: [line] }, revision },
  canUndo: false,
  canRedo: false,
});

/** One pen contact on the sheet, through the pointer path (no BOOX plugin in these tests). */
async function penTap(x: number, y: number) {
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
  for (const type of ["pointerdown", "pointerup"]) {
    const event = new Event(type, { bubbles: true, cancelable: true });
    Object.assign(event, {
      pointerId: 1,
      pointerType: "pen",
      clientX: x,
      clientY: y,
      pressure: 0.5,
      buttons: type === "pointerdown" ? 1 : 0,
      button: 0,
    });
    await act(async () => {
      canvas.dispatchEvent(event);
    });
  }
}

it("keeps one toolbar row and opens the eraser menu on the second tap only", async () => {
  await mount();

  // Every strip of chrome is sheet the pen cannot use; there is exactly one.
  expect(container.querySelectorAll('[role="toolbar"]')).toHaveLength(1);
  expect(document.querySelector("[data-ink-tool-options]")).toBeNull();

  // The first tap on a tool switches to it; only the second opens its options.
  await tap(button("Eraser"));
  expect(document.querySelector("[data-ink-tool-options]")).toBeNull();
  await tap(button("Eraser options"));
  expect(eraserMenu()).not.toBeNull();
  // The menu is not a second toolbar.
  expect(document.querySelectorAll('[role="toolbar"]')).toHaveLength(1);

  // A third tap closes it, and it stays closed.
  await tap(button("Eraser options"));
  expect(eraserMenu()).toBeNull();

  // The same toggle for the pen's widths.
  await tap(button("Pen"));
  await tap(button("Pen options"));
  expect(options("Pen width 5")).toBeTruthy();
  await tap(button("Pen options"));
  expect(document.querySelector("[data-ink-tool-options]")).toBeNull();
});

it("offers the three erasers in stock's order, as radios, and closes on a type", async () => {
  await mount();
  expect(toolbarEraserType()).toBe("stroke");
  await openEraserMenu();

  const radios = [...document.querySelectorAll<HTMLElement>('[data-eraser-menu] [role="radio"]')];
  expect(radios.map((radio) => radio.getAttribute("aria-label"))).toEqual([
    "Pixel Eraser",
    "Lasso Eraser",
    "Stroke Eraser",
  ]);
  // A pressed button would be painted solid black on e-ink; the choice is a dot instead.
  expect(radios.some((radio) => radio.hasAttribute("aria-pressed"))).toBe(false);
  expect(radios.map((radio) => radio.getAttribute("aria-checked"))).toEqual([
    "false",
    "false",
    "true",
  ]);
  expect(eraserMenu()?.textContent).toContain("Stroke Eraser 0.50mm");

  await tap(options("Pixel Eraser"));
  expect(eraserMenu()).toBeNull();
  expect(toolbarEraserType()).toBe("pixel");
  expect(lastCanvasProps()).toMatchObject({ eraserMode: "pixel", eraserDiameter: 500 });

  await tap(button("Eraser options"));
  expect(eraserMenu()?.textContent).toContain("Pixel Eraser 5.00mm");
  expect(eraserMenu()?.textContent).toContain("Eraser Diameter");

  await tap(options("Lasso Eraser"));
  await tap(button("Eraser options"));
  expect(eraserMenu()?.textContent).toContain("Lasso Eraser");
  expect(eraserMenu()?.textContent).not.toContain("Eraser Diameter");
  expect(document.querySelector('[data-eraser-menu] input[type="range"]')).toBeNull();

  // The selected type closes the menu too.
  await tap(options("Lasso Eraser"));
  expect(eraserMenu()).toBeNull();
});

it("steps the diameter with the menu open and hands it to the sheet once it closes", async () => {
  useEraserSettings.getState().setType("pixel");
  await mount();
  await openEraserMenu();

  await tap(options("Decrease eraser diameter"));
  await tap(options("Decrease eraser diameter"));
  expect(eraserMenu()?.textContent).toContain("Pixel Eraser 4.90mm");
  expect(useEraserSettings.getState().diameters.pixel).toBe(490);
  // Still open, and the sheet still erases with what it had when the menu opened.
  expect(eraserMenu()).not.toBeNull();
  expect(lastCanvasProps()).toMatchObject({ eraserMode: "pixel", eraserDiameter: 500 });

  const slider = document.querySelector<HTMLInputElement>(
    '[data-eraser-menu] input[type="range"]',
  )!;
  expect(slider.getAttribute("aria-valuetext")).toBe("4.90mm");
  await act(async () => {
    slider.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
  });
  expect(eraserMenu()?.textContent).toContain("Pixel Eraser 4.95mm");
  expect(useEraserSettings.getState().diameters.pixel).toBe(495);
  await act(async () => {
    slider.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
  });
  expect(eraserMenu()?.textContent).toContain("Pixel Eraser 8.00mm");
  // Nothing past the end.
  await tap(options("Increase eraser diameter"));
  expect(useEraserSettings.getState().diameters.pixel).toBe(800);

  // Stroke keeps its own diameter; pixel's comes back with it.
  await tap(options("Stroke Eraser"));
  expect(lastCanvasProps()).toMatchObject({ eraserMode: "stroke", eraserDiameter: 50 });
  await tap(button("Eraser options"));
  expect(eraserMenu()?.textContent).toContain("Stroke Eraser 0.50mm");
  await tap(options("Pixel Eraser"));
  expect(lastCanvasProps()).toMatchObject({ eraserMode: "pixel", eraserDiameter: 800 });
});

it("repaints a closed eraser menu without a full refresh", async () => {
  await mount();
  await openEraserMenu();

  // A type closes the menu by state; base-ui reports nothing for a controlled close.
  await tap(options("Pixel Eraser"));
  expect(eraserMenu()).toBeNull();
  await tap(button("Eraser options"));
  await tap(container.querySelector("canvas")!);
  expect(eraserMenu()).toBeNull();
  expect(refresh.soon).not.toHaveBeenCalled();

  // The pen's widths keep the full refresh for a press outside, and a width chosen from them,
  // which is also a controlled close, asks for none either.
  await tap(button("Pen"));
  await tap(button("Pen options"));
  await tap(options("Pen width 5"));
  expect(refresh.soon).not.toHaveBeenCalled();
  await tap(button("Pen options"));
  await tap(container.querySelector("canvas")!);
  expect(document.querySelector("[data-ink-tool-options]")).toBeNull();
  expect(refresh.soon).toHaveBeenCalledTimes(1);
});

it("erases the page after a confirmation, as one undoable write, and goes back to the pen", async () => {
  api.loadHandwritingNote.mockResolvedValue(inked("revision-1"));
  api.saveHandwritingPatch.mockResolvedValue("revision-2");
  await mount();
  await openEraserMenu();

  await tap(options("Erase Page"));
  expect(document.body.textContent).toContain(
    "Are you sure you want to erase strokes on this page?",
  );
  await tap(dialogButton("Cancel"));
  expect(api.saveHandwritingPatch).not.toHaveBeenCalled();
  expect(button("Eraser options").getAttribute("aria-pressed")).toBe("true");
  expect(eraserMenu()).not.toBeNull();

  await tap(options("Erase Page"));
  await tap(dialogButton("Erase"));
  expect(api.saveHandwritingPatch).toHaveBeenCalledTimes(1);
  expect(api.saveHandwritingPatch).toHaveBeenCalledWith(
    page.uuid,
    { order: [], upserts: [], background: "plain" },
    "revision-1",
  );
  expect(eraserMenu()).toBeNull();
  expect(button("Pen options").getAttribute("aria-pressed")).toBe("true");
  expect(button("Undo stroke").disabled).toBe(false);
});

it("confirms Erase Page on an empty page too, and writes nothing", async () => {
  await mount();
  await openEraserMenu();

  expect(options("Erase Page").hasAttribute("disabled")).toBe(false);
  await tap(options("Erase Page"));
  await tap(dialogButton("Erase"));
  expect(api.saveHandwritingPatch).not.toHaveBeenCalled();
  expect(eraserMenu()).toBeNull();
  expect(button("Pen options").getAttribute("aria-pressed")).toBe("true");
  expect(button("Undo stroke").disabled).toBe(true);
});

it("keeps Auto-deselect on the device and returns to the previous tool after an eraser erase", async () => {
  api.loadHandwritingNote.mockResolvedValue(inked("revision-1"));
  api.saveHandwritingPatch.mockResolvedValue("revision-2");
  await mount();
  await tap(button("Lasso"));
  await openEraserMenu();

  const toggle = document.querySelector<HTMLElement>('[data-eraser-menu] [role="switch"]')!;
  expect(toggle.getAttribute("aria-checked")).toBe("false");
  await tap(toggle);
  expect(toggle.getAttribute("aria-checked")).toBe("true");
  expect(eraserMenu()).not.toBeNull();
  expect(JSON.parse(localStorage.getItem("tangleaf.eraser-settings.v1")!).state.autoDeselect).toBe(
    true,
  );

  // The hint opens over the menu, and a tap elsewhere in the menu closes only the hint.
  const hint = "When enabled, auto-switch to last tool after erasing.";
  await tap(options("About Auto-deselect Eraser"));
  expect(document.body.textContent).toContain(hint);
  await tap(eraserMenu()!.querySelector("p")!);
  expect(document.body.textContent).not.toContain(hint);
  expect(eraserMenu()).not.toBeNull();

  await tap(button("Eraser options"));
  // The stroke eraser on the line at sheet (200, 100): the eraser tool hands back to the lasso.
  await penTap(100, 50);
  expect(api.saveHandwritingPatch).toHaveBeenCalledTimes(1);
  expect(button("Lasso options").getAttribute("aria-pressed")).toBe("true");
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
    "Pen options",
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

it("offers one pen whose second tap opens its stroke widths, at any bar width", async () => {
  await mount();

  expect(button.bind(null, "Pen, medium")).toThrow();
  expect(button("Pen options").getAttribute("aria-pressed")).toBe("true");
  await act(async () => {
    button("Eraser").click();
  });
  expect(button("Pen").getAttribute("aria-pressed")).toBe("false");
  await act(async () => {
    button("Pen").click();
  });
  expect(document.querySelector("[data-ink-tool-options]")).toBeNull();
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
