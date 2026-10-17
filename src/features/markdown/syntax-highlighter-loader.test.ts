import { describe, expect, it, vi } from "vite-plus/test";

describe("syntax highlighter loading failures", () => {
  it("keeps readable source after rejection and allows a later request to retry", async () => {
    vi.doMock("./syntax-highlighter-runtime", () => {
      throw new Error("chunk unavailable");
    });
    const policy = await import("./syntax-highlighter");
    const listener = vi.fn();
    const unsubscribe = policy.subscribeSyntaxHighlighter(listener);
    const source = 'fn main() {\n    println!("hello");\n}';
    try {
      await expect(policy.loadSyntaxHighlighter()).rejects.toThrow();
      expect(policy.cachedSyntaxHighlighter()).toBeNull();
      expect(listener).not.toHaveBeenCalled();
      const fallback = policy.highlightMarkdownCode(source, "rs");
      expect(fallback.highlighted).toBe(false);
      expect(
        fallback.lines.map((line) => line.map((token) => token.content).join("")).join("\n"),
      ).toBe(source);
      vi.doMock("./syntax-highlighter-runtime", () =>
        vi.importActual("./syntax-highlighter-runtime"),
      );
      const loaded = await policy.loadSyntaxHighlighter();
      expect(policy.cachedSyntaxHighlighter()).toBe(loaded);
      expect(listener).toHaveBeenCalledTimes(1);
      expect(policy.highlightMarkdownCode(source, "rs").highlighted).toBe(true);
    } finally {
      unsubscribe();
      vi.doUnmock("./syntax-highlighter-runtime");
    }
  });
});
