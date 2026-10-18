import { completeEveryNote, flushAllSessions } from "@/features/handwriting/handwriting-session";
import type { PageSessionRegistry } from "@/features/pages/page-session";
import { completeAllHandwriting } from "@/lib/api";

/**
 * Process-level drain of work that is queued but not yet stored.
 *
 * A window can go away without unmounting anything — Android kills a
 * backgrounded WebView outright — and a debounced autosave never fires then, so
 * the last chance to persist text and ink must not depend on a component being
 * mounted. This module is deliberately free of React: it is registered once
 * from the entry point and outlives every tree.
 */
let sessions: PageSessionRegistry | null = null;
let listening = false;

/** Explicit data tools must refuse to proceed while any primary draft is unsaved. */
export async function flushWorkspaceDrafts(pageSessions: PageSessionRegistry): Promise<void> {
  const results = await Promise.allSettled([pageSessions.flushAll(), flushAllSessions()]);
  if (results.some((result) => result.status === "rejected" || result.value === false)) {
    throw new Error(
      "Some notes have unsaved changes. Return to them and resolve saving before continuing.",
    );
  }
  // Unlike best-effort pagehide completion, this failure must reach the user.
  await completeAllHandwriting();
}

export function registerLifecycleFlush(pageSessions: PageSessionRegistry): void {
  sessions = pageSessions;
  if (listening) return;
  listening = true;
  const drain = () => {
    void sessions?.flushAll();
    void completeEveryNote();
  };
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") drain();
  });
  window.addEventListener("pagehide", drain);
}
