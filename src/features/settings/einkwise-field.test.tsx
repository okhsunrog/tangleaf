// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { EinkWiseField } from "./einkwise-field";
import { getEinkWiseStatus, setEinkWise, type EinkWiseStatus } from "@/lib/api";

vi.mock("@/lib/api", () => ({
  getEinkWiseStatus: vi.fn(),
  setEinkWise: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
});

/** The query and mutation results reach the component a timer later. */
const settle = () => act(async () => new Promise((resolve) => setTimeout(resolve, 0)));

async function mounted(status: EinkWiseStatus, run: (container: HTMLDivElement) => Promise<void>) {
  vi.mocked(getEinkWiseStatus).mockResolvedValue(status);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <EinkWiseField />
        </QueryClientProvider>,
      ),
    );
    await settle();
    await run(container);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
}

const checkbox = (container: HTMLElement) =>
  container.querySelector<HTMLInputElement>('input[type="checkbox"]');

it("is not shown where the firmware has no EinkWise", async () => {
  await mounted({ available: false, enabled: null, active: null }, async (container) => {
    expect(checkbox(container)).toBeNull();
  });
});

it("stores the choice and says it waits for a restart", async () => {
  vi.mocked(setEinkWise).mockResolvedValue({ available: true, enabled: false, active: true });
  await mounted({ available: true, enabled: true, active: true }, async (container) => {
    expect(checkbox(container)?.checked).toBe(true);
    expect(container.textContent).not.toContain("after the device restarts");
    await act(async () => checkbox(container)!.click());
    await settle();
    expect(setEinkWise).toHaveBeenCalledWith(false);
    expect(checkbox(container)?.checked).toBe(false);
    expect(container.textContent).toContain("Takes effect after the device restarts.");
  });
});

it("shows the enforced choice once the stored one has taken effect", async () => {
  await mounted({ available: true, enabled: false, active: false }, async (container) => {
    expect(checkbox(container)?.checked).toBe(false);
    expect(container.textContent).not.toContain("after the device restarts");
  });
});
