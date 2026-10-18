// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { listJournals, type Page } from "@/lib/api";
import { queryKeys } from "@/lib/query";
import { adjacentDisposition, currentDisposition } from "@/features/workspace/workspace-model";
import { JournalCalendar } from "./journal-calendar";

vi.mock("@/lib/api", async (original) => ({
  ...(await original<typeof import("@/lib/api")>()),
  listJournals: vi.fn(),
}));
vi.mock("./journal-date", async (original) => ({
  ...(await original<typeof import("./journal-date")>()),
  todayJournalDate: () => "2026-10-18",
}));

function journal(date: string): Page {
  return {
    uuid: date,
    kind: { kind: "journal", date },
    title: null,
    layout: "document",
    titleRevision: "1",
    createdAt: 0,
    updatedAt: 0,
  };
}

let root: Root;
let container: HTMLDivElement;
let client: QueryClient;
const onOpenDate = vi.fn();

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
  vi.mocked(listJournals).mockImplementation(async ({ beforeDate } = {}) =>
    ["2026-10-16", "2026-10-11", "2026-09-07", "2026-07-19"]
      .filter((date) => !beforeDate || date < beforeDate)
      .map(journal),
  );
  client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  client.clear();
  container.remove();
});

async function mount(activeDate = "2026-10-18", busy = false) {
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <JournalCalendar activeDate={activeDate} busy={busy} onOpenDate={onOpenDate} />
      </QueryClientProvider>,
    );
  });
}

function button(label: string) {
  const element = document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
  if (!element) throw new Error(`Missing button: ${label}`);
  return element;
}

function day(date: string) {
  const element = document.querySelector<HTMLButtonElement>(`[data-day="${date}"] button`);
  if (!element) throw new Error(`Missing day: ${date}`);
  return element;
}

async function click(element: HTMLButtonElement) {
  await act(async () => element.click());
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
}

it("only enables saved entries and today, and distinguishes entries from empty days", async () => {
  await mount();
  await click(button("Choose journal date"));
  expect(listJournals).toHaveBeenCalledWith({ beforeDate: "2026-11-01", limit: 31 });
  expect(day("2026-10-16").disabled).toBe(false);
  expect(day("2026-10-16").dataset.entry).toBe("true");
  expect(day("2026-10-16").getAttribute("aria-label")).toContain("journal entry");
  expect(day("2026-10-17").disabled).toBe(true);
  expect(day("2026-10-17").dataset.entry).toBeUndefined();
  expect(day("2026-10-19").disabled).toBe(true);
  expect(day("2026-10-18").disabled).toBe(false);
  expect(day("2026-10-18").dataset.today).toBe("true");
  expect(button("Next month").getAttribute("aria-disabled")).toBe("true");
  await click(day("2026-10-17"));
  await click(day("2026-10-19"));
  expect(onOpenDate).not.toHaveBeenCalled();
});

it("browses months without opening a day, then opens an existing older entry", async () => {
  await mount();
  await click(button("Choose journal date"));
  await click(button("Previous month"));
  expect(onOpenDate).not.toHaveBeenCalled();
  expect(listJournals).toHaveBeenLastCalledWith({ beforeDate: "2026-10-01", limit: 31 });
  expect(day("2026-09-07").dataset.entry).toBe("true");
  expect(day("2026-09-06").disabled).toBe(true);
  await click(day("2026-09-07"));
  expect(onOpenDate).toHaveBeenCalledExactlyOnceWith("2026-09-07", currentDisposition);
  expect(document.querySelector(".journal-calendar")).toBeNull();
});

it("keeps Shift-click opening in the adjacent pane", async () => {
  await mount();
  await click(button("Choose journal date"));
  await act(async () => {
    day("2026-10-11").dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
  });
  expect(onOpenDate).toHaveBeenCalledExactlyOnceWith("2026-10-11", adjacentDisposition);
});

it("allows starting today's journal without an existing entry", async () => {
  await mount();
  await click(button("Choose journal date"));
  await click(day("2026-10-18"));
  expect(onOpenDate).toHaveBeenCalledExactlyOnceWith("2026-10-18", currentDisposition);
});

it("does not reuse dates from the preceding month when month data is still loading", async () => {
  await mount();
  await click(button("Choose journal date"));
  vi.mocked(listJournals).mockReturnValue(new Promise(() => {}));
  await click(button("Previous month"));
  expect(day("2026-09-07").disabled).toBe(true);
  expect(document.querySelector('[role="status"]')?.textContent).toBeTruthy();
  expect(onOpenDate).not.toHaveBeenCalled();
});

it("updates selectable days after journal data is invalidated", async () => {
  await mount();
  await click(button("Choose journal date"));
  vi.mocked(listJournals).mockResolvedValue([journal("2026-10-17")]);
  await act(async () => {
    await client.invalidateQueries({ queryKey: queryKeys.journals });
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(day("2026-10-17").disabled).toBe(false);
  expect(day("2026-10-16").disabled).toBe(true);
});

it("offers retry on a failed month load without enabling empty dates", async () => {
  vi.mocked(listJournals).mockRejectedValueOnce(new Error("offline"));
  await mount();
  await click(button("Choose journal date"));
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("Could not load entries");
  expect(day("2026-10-16").disabled).toBe(true);
  const retry = [...document.querySelectorAll<HTMLButtonElement>("button")].find(
    (element) => element.textContent === "Retry",
  )!;
  await click(retry);
  expect(day("2026-10-16").disabled).toBe(false);
});
