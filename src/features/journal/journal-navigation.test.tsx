import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vite-plus/test";
import { JournalNavigation } from "./journal-navigation";

describe("JournalNavigation", () => {
  it("keeps date navigation explicit and accessible", () => {
    const html = renderToStaticMarkup(
      <JournalNavigation activeDate="2026-07-17" busy={false} onOpenDate={() => undefined} />,
    );

    expect(html).toContain("Journal");
    expect(html).toContain("Today");
    expect(html).not.toContain('aria-label="Open previous journal day"');
    expect(html).not.toContain('aria-label="Open next journal day"');
    expect(html).toContain('aria-label="Choose journal date"');
    expect(html).not.toContain('type="date"');
  });

  it("disables every date picker while a journal operation is in flight", () => {
    const html = renderToStaticMarkup(
      <JournalNavigation activeDate="2026-07-17" busy onOpenDate={() => undefined} />,
    );

    expect(html.match(/(?<!data-)disabled=""/g)).toHaveLength(2);
    expect(html).toContain('aria-busy="true"');
  });
});
