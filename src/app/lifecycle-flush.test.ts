// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vite-plus/test";
import {
  beginSession,
  resetHandwritingSessions,
  type SavePatch,
} from "@/features/handwriting/handwriting-session";
import { emptyDraft } from "@/features/handwriting/ink-model";
import { PageSessionRegistry } from "@/features/pages/page-session";
import { flushWorkspaceDrafts, registerLifecycleFlush } from "./lifecycle-flush";

const api = vi.hoisted(() => ({ completeAllHandwriting: vi.fn() }));

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return { ...actual, ...api };
});

afterEach(() => {
  resetHandwritingSessions();
  api.completeAllHandwriting.mockReset();
});

it("blocks explicit data tools while a retained text draft is unsaved", async () => {
  const sessions = new PageSessionRegistry();
  sessions.editTitle("page", "Unsaved", { text: "Saved", revision: "r1" });
  await expect(flushWorkspaceDrafts(sessions)).rejects.toThrow("unsaved changes");
  expect(api.completeAllHandwriting).not.toHaveBeenCalled();
  expect(sessions.getSnapshot("page").title?.draft).toBe("Unsaved");
});

it("does not proceed to native completion when a handwriting draft fails to save", async () => {
  const writer = beginSession(
    "019f0000-0000-7000-8000-00000000000a",
    { draft: emptyDraft(), revision: "revision-1" },
    () => {},
    vi.fn<SavePatch>().mockRejectedValue(new Error("disk unavailable")),
  );
  await writer.write({ ...emptyDraft(), background: "grid" });
  await expect(flushWorkspaceDrafts(new PageSessionRegistry())).rejects.toThrow("unsaved changes");
  expect(writer.hasPending()).toBe(true);
  expect(api.completeAllHandwriting).not.toHaveBeenCalled();
});

it("awaits completion and reports its failure to explicit data tools", async () => {
  const sessions = new PageSessionRegistry();
  const save = vi.fn().mockResolvedValue(true);
  sessions.registerFlush(save);
  api.completeAllHandwriting.mockRejectedValue(new Error("publication failed"));
  await expect(flushWorkspaceDrafts(sessions)).rejects.toThrow("publication failed");
  expect(save).toHaveBeenCalledTimes(1);
  api.completeAllHandwriting.mockResolvedValue(undefined);
  await expect(flushWorkspaceDrafts(sessions)).resolves.toBeUndefined();
});

it("drains a retained session when the window goes away with no view mounted", async () => {
  api.completeAllHandwriting.mockResolvedValue(undefined);
  registerLifecycleFlush(new PageSessionRegistry());
  const save = vi
    .fn<SavePatch>()
    .mockRejectedValueOnce(new Error("database is locked"))
    .mockResolvedValue("revision-2");
  const writer = beginSession(
    "019f0000-0000-7000-8000-00000000000a",
    { draft: emptyDraft(), revision: "revision-1" },
    () => {},
    save,
  );

  await writer.write({ ...emptyDraft(), background: "grid" });
  expect(writer.hasPending()).toBe(true);

  // Nothing is mounted; the process-level listener is the only thing that can
  // still store this gesture before the window is destroyed.
  window.dispatchEvent(new Event("pagehide"));

  await vi.waitFor(() => {
    expect(save).toHaveBeenCalledTimes(2);
    expect(api.completeAllHandwriting).toHaveBeenCalledTimes(1);
  });
});

it("stores a dirty text draft when the window is hidden", async () => {
  api.completeAllHandwriting.mockResolvedValue(undefined);
  const sessions = new PageSessionRegistry();
  registerLifecycleFlush(sessions);
  const save = vi.fn().mockResolvedValue(undefined);
  sessions.registerFlush(save);

  Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
  document.dispatchEvent(new Event("visibilitychange"));

  // Autosave is debounced, so without this the last keystrokes before the app
  // is backgrounded are never written.
  await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(1));
});
