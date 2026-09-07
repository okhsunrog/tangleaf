import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vite-plus/test";
import { RecentJournals } from "./recent-journals";

describe("RecentJournals", () => {
  const journal = {
    uuid: "019f0000-0000-7000-8000-000000000001",
    kind: { kind: "journal" as const, date: "2026-07-17" },
    title: null,
    layout: "outline" as const,
    titleRevision: "test-title-revision",
    createdAt: 0,
    updatedAt: 0,
  };

  it("prevents a second navigation while a journal request is pending", () => {
    const html = renderToStaticMarkup(
      <RecentJournals pages={[journal]} activeUuid={null} busy onOpen={() => undefined} />,
    );

    // Journal titles follow the reader's own locale, so the exact wording belongs to the runtime,
    // not to this test: pinning "Friday, July 17, 2026" only passed where the default locale
    // happened to be American. What is worth asserting is that the component asks for the long
    // spelled-out form rather than a numeric one.
    const expectedTitle = new Intl.DateTimeFormat(undefined, {
      weekday: "long",
      year: "numeric",
      month: "long",
      day: "numeric",
    }).format(new Date(2026, 6, 17));

    expect(html).toContain('disabled=""');
    expect(html).toContain(">17</span>");
    expect(html).toContain(`title="${expectedTitle}"`);
    expect(html).not.toContain("17.07");
  });

  it("fits recent days into the available width without a scrollbar", () => {
    const pages = Array.from({ length: 7 }, (_, index) => ({
      ...journal,
      uuid: `019f0000-0000-7000-8000-00000000000${index}`,
      kind: { kind: "journal" as const, date: `2026-07-${String(17 - index).padStart(2, "0")}` },
    }));
    const html = renderToStaticMarkup(
      <RecentJournals pages={pages} activeUuid={null} busy={false} onOpen={() => undefined} />,
    );

    expect(html).not.toContain("overflow-x-auto");
    expect(html).toContain("grid-cols-7");
    expect(html.match(/aria-label="Open journal /g)).toHaveLength(7);
  });
});
