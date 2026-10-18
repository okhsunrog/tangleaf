import { createInitialWorkspaceState } from "@/features/workspace/workspace-model";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vite-plus/test";
import { EmptyJournalView } from "./empty-journal-view";

vi.mock("./journal-entry", () => ({ JournalEntry: () => <div>Journal editor</div> }));

describe("EmptyJournalView", () => {
  it("previews a missing date without forcing the mobile keyboard open", () => {
    const html = renderToStaticMarkup(
      <EmptyJournalView
        date="2026-07-17"
        busy={false}
        paneId={createInitialWorkspaceState().primaryPaneId}
        onOpenDate={() => undefined}
      />,
    );

    expect(html).not.toContain("Save first entry");
    expect(html).toContain("Journal editor");
    expect(html).not.toContain("autofocus");
  });

  it("exposes its pending state and disables capture controls", () => {
    const html = renderToStaticMarkup(
      <EmptyJournalView
        date="2026-07-17"
        busy
        paneId={createInitialWorkspaceState().primaryPaneId}
        onOpenDate={() => undefined}
      />,
    );

    expect(html.match(/disabled=""/g)?.length).toBeGreaterThanOrEqual(2);
  });
});
