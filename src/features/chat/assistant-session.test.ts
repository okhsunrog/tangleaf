import { describe, expect, it } from "vitest";
import {
  assistantSessionReducer,
  boundedAssistantHistory,
  createAssistantSession,
} from "./assistant-session";

describe("Assistant session controller state", () => {
  it("preserves input independently from any dock host", () => {
    const state = assistantSessionReducer(createAssistantSession(), {
      type: "set_input",
      input: "draft question",
    });

    expect(state.input).toBe("draft question");
    expect(state.busy).toBe(false);
  });

  it("continues applying stream events while its view is absent", () => {
    let state = assistantSessionReducer(createAssistantSession(), {
      type: "start",
      message: "question",
      requestId: "request-1",
    });
    state = assistantSessionReducer(state, {
      type: "event",
      event: { kind: "text_delta", text: "answer" },
    });

    expect(state.busy).toBe(true);
    expect(state.activeRequestId).toBe("request-1");
    expect(state.turns[state.turns.length - 1]?.text).toBe("answer");
  });

  it("ignores a stale completion and retains cancellation output", () => {
    let state = assistantSessionReducer(createAssistantSession(), {
      type: "start",
      message: "question",
      requestId: "request-1",
    });
    state = assistantSessionReducer(state, { type: "finish", requestId: "stale" });
    expect(state.busy).toBe(true);

    state = assistantSessionReducer(state, {
      type: "event",
      event: { kind: "cancelled" },
    });
    state = assistantSessionReducer(state, { type: "finish", requestId: "request-1" });
    expect(state.busy).toBe(false);
    expect(state.turns[state.turns.length - 1]?.text).toContain("Stopped");
  });

  it("retains source snapshots in stored turns while sending only the original text to the model", () => {
    let state = assistantSessionReducer(createAssistantSession(), {
      type: "start",
      message: "charging?",
      requestId: "r",
    });
    state = assistantSessionReducer(state, {
      type: "event",
      event: {
        kind: "sources",
        sources: [
          { target: { kind: "page", uuid: "source-id" }, title: "Charging", snippet: "BQ25756" },
        ],
      },
    });
    state = assistantSessionReducer(state, {
      type: "event",
      event: { kind: "text_delta", text: "BQ25756 [@source-id]" },
    });
    const restored = createAssistantSession(JSON.parse(JSON.stringify(state.turns)));
    expect(restored.turns[1]?.sources?.[0]?.title).toBe("Charging");
    expect(boundedAssistantHistory(restored.turns)).toEqual([
      { role: "user", text: "charging?" },
      { role: "assistant", text: "BQ25756 [@source-id]" },
    ]);
  });

  it("ignores a stale legacy resolution after clearing the conversation", () => {
    const turn = { role: "assistant", text: "old" } as const;
    const state = assistantSessionReducer(createAssistantSession(), {
      type: "resolve_sources",
      turn,
      sources: [],
    });
    expect(state.turns).toEqual([]);
  });
});
