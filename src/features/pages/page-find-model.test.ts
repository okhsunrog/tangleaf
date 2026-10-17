import { describe, expect, it } from "vite-plus/test";
import { findTextMatches } from "./page-find-model";

describe("find in note", () => {
  it("finds repeated Cyrillic matches without changing source offsets", () => {
    expect(findTextMatches("Шкаф и шкаф", "шкаф", false)).toEqual([
      { from: 0, to: 4 },
      { from: 7, to: 11 },
    ]);
    expect(findTextMatches("Шкаф и шкаф", "шкаф", true)).toEqual([{ from: 7, to: 11 }]);
  });
  it("treats code and link punctuation literally", () => {
    expect(findTextMatches("a.* axy a.*", "a.*", false)).toEqual([
      { from: 0, to: 3 },
      { from: 8, to: 11 },
    ]);
    expect(findTextMatches("[[rust]]", "[[", false)).toEqual([{ from: 0, to: 2 }]);
  });
  it("handles empty queries and UTF-16 offsets", () => {
    expect(findTextMatches("hello", "", false)).toEqual([]);
    expect(findTextMatches("💡test", "test", false)).toEqual([{ from: 2, to: 6 }]);
  });
});
