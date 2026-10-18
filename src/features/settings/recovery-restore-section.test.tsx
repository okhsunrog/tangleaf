// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { PageSessionProvider } from "@/features/pages/page-session";
import { createAppQueryClient } from "@/lib/query";
import { dismissBackOverlay } from "@/lib/back-overlays";
import { RecoveryRestoreSection } from "./recovery-restore-section";

const mocks = vi.hoisted(() => ({
  chooseRecoveryRestore: vi.fn(),
  commitRecoveryRestore: vi.fn(),
  discardRecoveryRestore: vi.fn(),
  listRecoveryBackups: vi.fn(),
  prepareRecoveryRestore: vi.fn(),
  restartApp: vi.fn(),
  flushWorkspaceDrafts: vi.fn(),
  confirm: vi.fn(),
}));
vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  ...mocks,
}));
vi.mock("@/app/lifecycle-flush", () => ({ flushWorkspaceDrafts: mocks.flushWorkspaceDrafts }));
vi.mock("@/app/confirmation", () => ({ useConfirmation: () => mocks.confirm }));

const preview = {
  token: "019f0000-0000-7000-8000-000000000001",
  createdAt: 1_792_272_000,
  workspaceUuid: "019f0000-0000-7000-8000-000000000002",
  currentWorkspaceUuid: "019f0000-0000-7000-8000-000000000003",
  workspaceDiffers: true,
  commitVersion: 42,
  schemaVersion: 11,
  databaseBytes: 1_048_576,
  blobCount: 3,
  blobBytes: 600,
  pageCount: 12,
  blockCount: 34,
};

beforeEach(() => {
  vi.resetAllMocks();
  mocks.listRecoveryBackups.mockResolvedValue([]);
  mocks.chooseRecoveryRestore.mockResolvedValue(preview);
  mocks.discardRecoveryRestore.mockResolvedValue(undefined);
  mocks.flushWorkspaceDrafts.mockResolvedValue(undefined);
  mocks.confirm.mockResolvedValue(true);
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
});

async function mounted(
  run: (container: HTMLDivElement, busy: ReturnType<typeof vi.fn>) => Promise<void>,
) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const client = createAppQueryClient();
  const busy = vi.fn();
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <PageSessionProvider>
            <RecoveryRestoreSection disabled={false} dataAvailable onBusyChange={busy} />
          </PageSessionProvider>
        </QueryClientProvider>,
      ),
    );
    await run(container, busy);
  } finally {
    await act(async () => root.unmount());
    client.clear();
    container.remove();
  }
}

function button(text: string) {
  const found = [...document.querySelectorAll("button")].find((element) =>
    element.textContent?.includes(text),
  );
  expect(found, `button ${text}`).toBeDefined();
  return found!;
}
async function click(text: string) {
  await act(async () => button(text).click());
}

it("shows the verified backup before confirmation and discards a canceled preview", async () => {
  await mounted(async (container) => {
    await click("Choose folder");
    expect(container.textContent).toContain("12 pages · 34 blocks · 3 files");
    expect(container.textContent).toContain("another workspace");
    expect(mocks.commitRecoveryRestore).not.toHaveBeenCalled();
    await click("Cancel restore");
    expect(mocks.discardRecoveryRestore).toHaveBeenCalledWith(preview.token);
    expect(container.querySelector('[aria-label="Recovery backup preview"]')).toBeNull();
  });
});

it("does not flush or commit when the final replacement confirmation is declined", async () => {
  mocks.confirm.mockResolvedValue(false);
  await mounted(async () => {
    await click("Choose folder");
    await click("Restore backup");
    expect(mocks.confirm).toHaveBeenCalledWith(expect.objectContaining({ destructive: true }));
    expect(mocks.flushWorkspaceDrafts).not.toHaveBeenCalled();
    expect(mocks.commitRecoveryRestore).not.toHaveBeenCalled();
  });
});

it("retains the preview and refuses replacement if a draft cannot be saved", async () => {
  mocks.flushWorkspaceDrafts.mockRejectedValue(new Error("Unsaved draft"));
  await mounted(async (container) => {
    await click("Choose folder");
    await click("Restore backup");
    expect(container.textContent).toContain("Unsaved draft");
    expect(container.querySelector('[aria-label="Recovery backup preview"]')).not.toBeNull();
    expect(mocks.commitRecoveryRestore).not.toHaveBeenCalled();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });
});

it("flushes before commit and keeps a drained workspace behind the restart dialog", async () => {
  mocks.commitRecoveryRestore.mockResolvedValue({
    status: "restored",
    restartRequired: true,
    beforeRestoreBackupPath: "/test/before.recovery",
    error: null,
    warning: null,
  });
  await mounted(async (_, busy) => {
    await click("Choose folder");
    await click("Restore backup");
    expect(mocks.flushWorkspaceDrafts.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.commitRecoveryRestore.mock.invocationCallOrder[0],
    );
    expect(mocks.commitRecoveryRestore).toHaveBeenCalledWith(preview.token);
    expect(document.body.textContent).toContain("Backup restored");
    expect(document.body.textContent).toContain("/test/before.recovery");
    expect(busy).toHaveBeenLastCalledWith(true);
    expect(dismissBackOverlay()).toBe(true);
    const dialog = document.querySelector('[role="dialog"]')!;
    await act(async () =>
      dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })),
    );
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
    await click("Restart app");
    expect(mocks.restartApp).toHaveBeenCalledTimes(1);
  });
  expect(dismissBackOverlay()).toBe(false);
});

it("does not claim success when publication happened but fsync failed", async () => {
  mocks.commitRecoveryRestore.mockResolvedValue({
    status: "published_durability_uncertain",
    restartRequired: true,
    beforeRestoreBackupPath: "/test/before.recovery",
    error: "Directory sync failed",
    warning: null,
  });
  await mounted(async () => {
    await click("Choose folder");
    await click("Restore backup");
    expect(document.body.textContent).toContain("durability could not be confirmed");
    expect(document.body.textContent).toContain("Directory sync failed");
    expect(document.body.textContent).not.toContain("Backup restored");
  });
});

it("requires restart after an ambiguous commit transport failure", async () => {
  mocks.commitRecoveryRestore.mockRejectedValue(new Error("IPC disconnected"));
  await mounted(async () => {
    await click("Choose folder");
    await click("Restore backup");
    expect(document.body.textContent).toContain("Restore result could not be confirmed");
    expect(document.body.textContent).toContain("IPC disconnected");
    expect(button("Restart app").disabled).toBe(false);
    expect(document.body.textContent).not.toContain("Backup restored");
  });
});
