// @vitest-environment jsdom

import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vite-plus/test";
import { MarkdownRenderer } from "./markdown-renderer";
import { cachedSyntaxHighlighter, loadSyntaxHighlighter } from "./syntax-highlighter";

const pending = vi.hoisted(() => {
  let release!: () => void;
  return {
    ready: new Promise<void>((resolve) => {
      release = resolve;
    }),
    release: () => release(),
    imports: vi.fn(),
    display: "standard" as "standard" | "eink",
    ink: "color" as "color" | "mono",
  };
});

vi.mock("./syntax-highlighter-runtime", async (importOriginal) => {
  pending.imports();
  await pending.ready;
  return importOriginal();
});
vi.mock("./mermaid-diagram", () => ({
  MermaidDiagram: ({ source }: { source: string }) => <pre>{source}</pre>,
}));
vi.mock("@/app/appearance", () => ({
  useResolvedDisplay: () => pending.display,
  useResolvedInkColor: () => pending.ink,
}));

vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);

const context = { kind: "assistant" } as const;

const roots: Array<ReturnType<typeof createRoot>> = [];
afterEach(() => {
  for (const root of roots.splice(0)) act(() => root.unmount());
});

describe("on-demand code highlighting", () => {
  it("leaves ordinary, inline, unsupported and oversized code outside the runtime", async () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    roots.push(root);
    await act(async () =>
      root.render(
        <MarkdownRenderer
          context={context}
          markdown={[
            "Ordinary **Markdown** and `const inline = 1`.",
            "```text\nplain text\n```",
            "```unknown\nunknown language\n```",
            "```constructor\nnot a language\n```",
            "```mermaid\nflowchart LR\nA --> B\n```",
            "```\nunlabelled code\n```",
            `\`\`\`rust\n${"x".repeat(4_097)}\n\`\`\``,
            `\`\`\`rust\n${"x\n".repeat(201)}\`\`\``,
            `\`\`\`rust\n${("x".repeat(2_000) + "\n").repeat(17)}\`\`\``,
          ].join("\n\n")}
        />,
      ),
    );
    expect(container.textContent).toContain("Ordinary Markdown");
    expect(pending.imports).not.toHaveBeenCalled();
    expect(cachedSyntaxHighlighter()).toBeNull();
  });

  it("shares loading, preserves code/copy/focus and uses the latest source and theme", async () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    roots.push(root);
    document.body.append(container);
    const source = 'fn main() {\n    println!("hello");\n}';
    const updated = "fn main() {\n    // updated\n}";
    const render = (code: string) => (
      <>
        <MarkdownRenderer context={context} markdown={`\`\`\`rs\n${code}\n\`\`\``} />
        <MarkdownRenderer context={context} markdown="```json\n{}\n```" />
        <div id="block-target" tabIndex={-1}>
          Target block
        </div>
      </>
    );
    const copy = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: copy },
    });
    await act(async () => root.render(render(source)));
    const pre = container.querySelector("pre")!;
    const lines = [...pre.querySelectorAll(".markdown-code-line")];
    const button = container.querySelector("button")!;
    const target = container.querySelector<HTMLElement>("#block-target")!;
    target.focus();
    container.scrollTop = 150;
    expect(pre.textContent).toBe(source);
    expect(lines).toHaveLength(3);
    expect(container.querySelector('[data-code-highlighted="true"]')).toBeNull();
    await act(async () => button.click());
    expect(copy).toHaveBeenCalledWith(source);
    expect(pending.imports).toHaveBeenCalledTimes(1);
    expect(loadSyntaxHighlighter()).toBe(loadSyntaxHighlighter());

    pending.display = "eink";
    pending.ink = "mono";
    await act(async () => root.render(render(updated)));
    await act(async () => {
      pending.release();
      await loadSyntaxHighlighter();
    });
    expect(container.querySelector("pre")).toBe(pre);
    expect([...pre.querySelectorAll(".markdown-code-line")]).toEqual(lines);
    expect(pre.textContent).toBe(updated);
    expect(
      container.querySelector('[data-code-language="rust"][data-code-highlighted="true"]'),
    ).not.toBeNull();
    expect(pre.querySelector('[style*="font-weight: 600"]')).not.toBeNull();
    expect(pre.querySelector('[style*="--shiki-light: #555555"]')).not.toBeNull();
    expect(document.activeElement).toBe(target);
    expect(container.scrollTop).toBe(150);
    expect(container.querySelector("button")).toBe(button);

    pending.display = "standard";
    pending.ink = "color";
    await act(async () => root.render(render(updated)));
    expect(pre.querySelector('[style*="font-weight: 600"]')).toBeNull();
    expect(pre.querySelector('[style*="--shiki-dark"]')).not.toBeNull();
    await act(async () => button.click());
    expect(copy).toHaveBeenLastCalledWith(updated);
    container.remove();
  });
});
