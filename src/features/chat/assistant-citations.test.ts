import { describe, expect, it } from "vite-plus/test";
import type { ChatSource } from "@/lib/api";
import { assistantCitationMarkers, prepareAssistantCitations } from "./assistant-citations";

const PAGE = "019c8d1a-4ab1-7f31-8f00-f594337c3ca5";
const BLOCK = "019c8d1a-4ab1-7f31-8f00-f594337c3ca6";
const UNKNOWN = "019c8d1a-4ab1-7f31-8f00-f594337c3ca7";
const sources: ChatSource[] = [
  { target: { kind: "page", uuid: PAGE }, title: "Charging", snippet: "" },
  { target: { kind: "block", uuid: BLOCK, pageUuid: PAGE }, title: "Charging", snippet: "BQ25756" },
];

describe("assistant citations", () => {
  it("numbers cited targets in reading order and reuses numbers across legacy and new markers", () => {
    const result = prepareAssistantCitations(
      `BQ25756 [@${BLOCK}]\n\nSee [${PAGE.toUpperCase()}] and [${BLOCK}].`,
      sources,
    );
    expect(result.citations.map((source) => [source.target.uuid, source.number])).toEqual([
      [BLOCK, 1],
      [PAGE, 2],
    ]);
    expect(result.markdown.match(/\\\[1\\\]/g)).toHaveLength(2);
    expect(result.markdown).toContain(`notes-block:${BLOCK}`);
    expect(result.markdown).toContain(`notes-page-id:${PAGE}`);
    expect(prepareAssistantCitations(`[@${BLOCK}]`, sources).citations).toHaveLength(1);
  });

  it("does not reinterpret code, existing links, images, wikilinks or escaped markers", () => {
    const markdown = [
      `\`[@${BLOCK}]\``,
      `~~~text\n[${BLOCK}]\n~~~`,
      `    [${BLOCK}]`,
      `[label [${BLOCK}]](https://example.com)`,
      `![image [${BLOCK}]](notes-attachment:${PAGE})`,
      `[[${BLOCK}]]`,
      `\\[${BLOCK}]`,
      `&#91;${BLOCK}]`,
    ].join("\n\n");
    expect(assistantCitationMarkers(markdown)).toEqual([]);
    expect(prepareAssistantCitations(markdown, sources).markdown).toBe(markdown);
  });

  it("preserves source offsets around entities, Unicode, lists and GFM tables", () => {
    const markdown = `Русский &amp; текст **[@${BLOCK}]**\n\n- item [${PAGE}]\n\n| Note |\n| --- |\n| [@${BLOCK}] |`;
    expect(
      assistantCitationMarkers(markdown).map((marker) => markdown.slice(marker.from, marker.to)),
    ).toEqual([`[@${BLOCK}]`, `[${PAGE}]`, `[@${BLOCK}]`]);
  });

  it("keeps unverified IDs inert and compact even when they exist outside the registry", () => {
    const result = prepareAssistantCitations(`Unknown [@${UNKNOWN}]`, sources);
    expect(result.markdown).toBe("Unknown \\[source unavailable\\]");
    expect(result.citations).toEqual([]);
    expect(prepareAssistantCitations(`[${UNKNOWN}]`, [], undefined, true).markdown).toBe(
      "\\[source…\\]",
    );
  });
});
