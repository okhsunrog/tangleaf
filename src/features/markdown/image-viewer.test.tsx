// @vitest-environment jsdom

import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vite-plus/test";
import { MarkdownImageViewer } from "./image-viewer";
import { dismissBackOverlay } from "@/lib/back-overlays";

it("marks the portaled viewer as fullscreen and loads the original only when open", async () => {
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <MarkdownImageViewer
          alt="Photo"
          image={{
            src: "http://localhost/preview",
            originalSrc: "http://localhost/original",
            width: 1024,
            height: 771,
            byteSize: 1000,
            mime: "image/png",
          }}
        />,
      ),
    );
    expect(document.querySelector('[src="http://localhost/original"]')).toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
    const dialog = document.querySelector('[role="dialog"]');
    expect(dialog?.getAttribute("data-fullscreen")).toBe("true");
    expect(dialog?.classList.contains("bg-black")).toBe(true);
    expect(dialog?.querySelector('[src="http://localhost/original"]')).not.toBeNull();
    expect(container.contains(dialog)).toBe(false);
    const original = dialog!.querySelector<HTMLImageElement>('[src="http://localhost/original"]')!;
    for (const image of document.querySelectorAll("img")) {
      expect(image.crossOrigin).toBe("anonymous");
    }
    let finishDecode!: () => void;
    original.decode = () =>
      new Promise<void>((resolve) => {
        finishDecode = resolve;
      });
    await act(async () => original.dispatchEvent(new Event("load")));
    expect(original.style.visibility).toBe("hidden");
    expect(dialog!.querySelector('[src="http://localhost/preview"]')).not.toBeNull();
    await act(async () => finishDecode());
    expect(original.style.visibility).toBe("visible");
    expect(original.style.transition).toBe("");
    expect(dialog!.querySelector('[src="http://localhost/preview"]')).toBeNull();
    await act(async () =>
      document.querySelector<HTMLButtonElement>('[aria-label="Zoom in"]')!.click(),
    );
    expect(
      document.querySelector<HTMLImageElement>('[src="http://localhost/original"]')!.style
        .transform,
    ).toContain("scale(1.25)");
    await act(async () =>
      document.querySelector<HTMLButtonElement>('[aria-label="Close image viewer"]')!.click(),
    );
    expect(document.querySelector('[src="http://localhost/original"]')).toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
    expect(
      document.querySelector<HTMLImageElement>('[src="http://localhost/original"]')!.style
        .transform,
    ).toBe("translate3d(0px, 0px, 0) scale(1)");
    await act(async () => {
      expect(dismissBackOverlay()).toBe(true);
    });
    expect(document.querySelector('[src="http://localhost/original"]')).toBeNull();
    expect(dismissBackOverlay()).toBe(false);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});

const image = {
  src: "http://localhost/preview",
  originalSrc: "http://localhost/original",
  width: 1024,
  height: 771,
  byteSize: 1000,
  mime: "image/png" as const,
};

async function withViewer(
  run: (container: HTMLDivElement, root: ReturnType<typeof createRoot>) => Promise<void>,
) {
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<MarkdownImageViewer alt="Photo" image={image} />));
    await run(container, root);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
}

it("replaces a failed preview with an actionable placeholder and can still open the original", () =>
  withViewer(async (container) => {
    await act(async () => container.querySelector("img")!.dispatchEvent(new Event("error")));
    expect(container.textContent).toContain("Preview unavailable");
    expect(container.querySelector("img")).toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
    const dialog = document.querySelector('[role="dialog"]')!;
    // Opening also retries the failed thumbnail; it may fail independently again.
    expect(container.querySelector("img")).not.toBeNull();
    await act(async () => container.querySelector("img")!.dispatchEvent(new Event("error")));
    expect(dialog.textContent).toContain("Loading image");
    const original = dialog.querySelector<HTMLImageElement>('[src="http://localhost/original"]')!;
    original.decode = async () => {};
    await act(async () => original.dispatchEvent(new Event("load")));
    expect(original.style.visibility).toBe("visible");
    expect(dialog.textContent).not.toContain("Loading image");
  }));

it("keeps a preview on original failure and retries with a fresh image element", () =>
  withViewer(async (container) => {
    await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
    const dialog = document.querySelector('[role="dialog"]')!;
    const original = dialog.querySelector<HTMLImageElement>('[src="http://localhost/original"]')!;
    await act(async () => original.dispatchEvent(new Event("error")));
    expect(dialog.textContent).toContain("Could not load the full image.");
    expect(dialog.querySelector('[src="http://localhost/preview"]')).not.toBeNull();
    const retry = [...dialog.querySelectorAll("button")].find(
      (button) => button.textContent === "Retry",
    )!;
    await act(async () => retry.click());
    expect(dialog.textContent).not.toContain("Could not load");
    const retried = dialog.querySelector<HTMLImageElement>('[src="http://localhost/original"]')!;
    expect(retried).not.toBe(original);
    retried.decode = async () => {};
    await act(async () => retried.dispatchEvent(new Event("load")));
    expect(retried.style.visibility).toBe("visible");
    expect(dialog.querySelector('[src="http://localhost/preview"]')).toBeNull();
  }));

it("offers retry when decoding the original fails", () =>
  withViewer(async (container) => {
    await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
    const dialog = document.querySelector('[role="dialog"]')!;
    const original = dialog.querySelector<HTMLImageElement>('[src="http://localhost/original"]')!;
    original.decode = async () => {
      throw new Error("Decode failed");
    };
    await act(async () => original.dispatchEvent(new Event("load")));
    expect(dialog.textContent).toContain("Could not load the full image.");
    expect(original.style.visibility).toBe("hidden");
  }));

it("does not carry a failed preview over to a changed attachment", () =>
  withViewer(async (container, root) => {
    await act(async () => container.querySelector("img")!.dispatchEvent(new Event("error")));
    await act(async () =>
      root.render(
        <MarkdownImageViewer
          alt="Other"
          image={{
            ...image,
            src: "http://localhost/other-preview",
            originalSrc: "http://localhost/other-original",
          }}
        />,
      ),
    );
    expect(container.textContent).not.toContain("Preview unavailable");
    expect(container.querySelector("img")!.src).toBe("http://localhost/other-preview");
  }));
