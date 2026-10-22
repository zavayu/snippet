import { describe, expect, it } from "vitest";
import { conversationReducer as reduce, type ConversationState, type TurnIdentity } from "./conversation";

const identity: TurnIdentity = { conversationId: 1, turnId: 1 };
const event = { requestId: 10, context: identity };
const empty: ConversationState = { conversationId: 1, expanded: false, turns: [] };

function streaming() {
  let state = reduce(empty, { type: "submit", identity, prompt: "Explain this", kind: "initial", reference: "Screenshot attached" });
  state = reduce(state, { type: "started", event });
  return reduce(state, { type: "chunk", event, delta: "First answer" });
}

describe("conversation lifecycle", () => {
  it("expands without submitting and retains earlier answers during follow-up streaming", () => {
    let state = reduce(streaming(), { type: "complete", event });
    state = reduce(state, { type: "expand" });
    expect(state.expanded).toBe(true);
    expect(state.turns).toHaveLength(1);
    state = reduce(state, { type: "submit", identity: { ...identity, turnId: 2 }, kind: "follow-up", prompt: "Why?" });
    const nextEvent = { requestId: 11, context: { ...identity, turnId: 2 } };
    state = reduce(state, { type: "started", event: nextEvent });
    state = reduce(state, { type: "chunk", event: nextEvent, delta: "Because…" });
    expect(state.turns.map((turn) => turn.response)).toEqual(["First answer", "Because…"]);
    expect(state.turns[0].reference).toBe("Screenshot attached");
  });

  it("resets to the compact empty state and ignores every late event from the old conversation", () => {
    let state = reduce(streaming(), { type: "reset", conversationId: 2 });
    for (const action of [
      { type: "started", event }, { type: "chunk", event, delta: "Late text" },
      { type: "complete", event }, { type: "error", identity, message: "Old failure" },
    ] as const) state = reduce(state, action);
    expect(state).toEqual({ conversationId: 2, expanded: false, turns: [] });
  });

  it("does not let a delayed command acknowledgement restart a completed reply", () => {
    const complete = reduce(streaming(), { type: "complete", event });
    expect(reduce(complete, { type: "started", event })).toBe(complete);
    expect(reduce(complete, { type: "chunk", event, delta: "late" })).toBe(complete);
  });

  it("keeps partial stopped responses and replaces only that turn when retrying", () => {
    const stopped = reduce(streaming(), { type: "stopped", event });
    expect(stopped.turns[0]).toMatchObject({ status: "stopped", response: "First answer" });
    const retried = reduce(stopped, { type: "submit", identity: { ...identity, turnId: 2 }, kind: "initial", prompt: "Explain this", retry: true });
    expect(retried.turns).toHaveLength(1);
    expect(retried.turns[0]).toMatchObject({ id: 2, status: "loading", response: "" });
    expect(reduce(retried, { type: "started", event })).toBe(retried);
  });

  it("rejects double submission and events for another request or turn", () => {
    const state = streaming();
    expect(reduce(state, { type: "submit", identity: { ...identity, turnId: 2 }, prompt: "Duplicate", kind: "follow-up" })).toBe(state);
    expect(reduce(state, { type: "chunk", event: { ...event, requestId: 20 }, delta: "Wrong request" })).toBe(state);
    expect(reduce(state, { type: "chunk", event: { ...event, context: { ...identity, turnId: 2 } }, delta: "Wrong turn" })).toBe(state);
    expect(reduce(state, { type: "started", event: { requestId: 20 } })).toBe(state);
  });

  it("preserves streamed content on failure and leaves completed earlier turns intact", () => {
    let state = reduce(streaming(), { type: "complete", event });
    const nextIdentity = { ...identity, turnId: 2 };
    const nextEvent = { requestId: 11, context: nextIdentity };
    state = reduce(state, { type: "submit", identity: nextIdentity, kind: "follow-up", prompt: "Why?" });
    state = reduce(state, { type: "started", event: nextEvent });
    state = reduce(state, { type: "chunk", event: nextEvent, delta: "Partial" });
    state = reduce(state, { type: "error", identity: nextIdentity, message: "Ollama unavailable" });
    expect(state.turns[0].status).toBe("complete");
    expect(state.turns[1]).toMatchObject({ status: "error", response: "Partial", error: "Ollama unavailable" });
  });
});
