import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { MarkdownResponse } from "./components/MarkdownResponse";
import { ConversationPanel } from "./components/ConversationPanel";
import { responseWithSources, safeWebUrl, type ResearchInfo, type ResearchSource } from "./research";
import { conversationReducer, type ConversationState } from "./conversation";

const sources: ResearchSource[] = [{ id: 1, title: "Official docs", url: "https://example.com/docs", excerpt: "Evidence", publishedAt: null }];
const research: ResearchInfo = { phase: "summarizing", query: "test", sources };

describe("research citations", () => {
  it("links every known source in a grouped citation while preserving order and unknown IDs", () => {
    const grouped = [1, 2, 4, 5].map((id) => ({ ...sources[0], id, url: `https://example.com/${id}` }));
    const html = renderToStaticMarkup(createElement(MarkdownResponse, {
      content: "Finding [2, 4, 5, 1]. Partial [2, 99]. Code `[2, 4]`.", sources: grouped,
    }));
    expect([...html.matchAll(/href="([^"]+)"/g)].map((match) => match[1])).toEqual([
      "https://example.com/2", "https://example.com/4", "https://example.com/5", "https://example.com/1", "https://example.com/2",
    ]);
    expect(html).toContain(", 99]");
    expect(html).toContain("<code>[2, 4]</code>");
  });
  it("links only retrieved source IDs and leaves code unchanged", () => {
    const html = renderToStaticMarkup(createElement(MarkdownResponse, { content: "Finding [1]. Unknown [99]. Code `[1]`.", sources }));
    expect(html).toContain('href="https://example.com/docs"');
    expect(html.match(/href=/g)).toHaveLength(1);
    expect(html).toContain("[99]");
    expect(html).toContain("<code>[1]</code>");
  });
  it("does not render invented URLs or remote images from research output", () => {
    const html = renderToStaticMarkup(createElement(MarkdownResponse, {
      content: "[invented](https://invented.example/) ![image](https://tracker.example/pixel)", sources,
    }));
    expect(html).not.toContain("href=");
    expect(html).not.toContain("<img");
    expect(safeWebUrl("javascript:alert(1)")).toBeNull();
    expect(safeWebUrl("https://user:password@example.com/")).toBeNull();
  });
  it("preserves source URLs when copying or replacing a researched answer", () => {
    expect(responseWithSources("Finding [1]", sources)).toContain("[1] Official docs - https://example.com/docs");
    expect(responseWithSources("Ordinary answer")).toBe("Ordinary answer");
  });
});

describe("research presentation", () => {
  const render = (status: "loading" | "streaming" | "complete") => renderToStaticMarkup(createElement(ConversationPanel, {
    conversation: { conversationId: 1, expanded: false, turns: [{
      id: 1, kind: "initial", prompt: "Research", status, research,
      response: status === "loading" ? "" : "Finding [1].",
    }] },
    replacementId: null, onReplacementUsed: () => {}, onContinue: () => {}, onNewConversation: () => {},
    onSubmit: () => {}, onRetry: () => {}, onStop: () => {},
  }));
  it("keeps search progress and sources out of the response while generating", () => {
    for (const status of ["loading", "streaming"] as const) {
      const html = render(status);
      expect(html).not.toContain("<details");
      expect(html).not.toContain("Search:");
      expect(html).not.toContain("Summarizing sources");
      expect(html).not.toContain('role="status"');
    }
    expect(render("streaming")).toContain('href="https://example.com/docs"');
  });
  it("shows completed sources in a disclosure collapsed by default", () => {
    const html = render("complete");
    expect(html).toContain("<details");
    expect(html).toContain("Sources (1)");
    expect(html).toContain("Search: test");
    expect(html).not.toMatch(/<details[^>]*\bopen(?:=|\s|>)/);
  });
});

describe("research conversation lifecycle", () => {
  const identity = { conversationId: 1, turnId: 1 };
  const initial: ConversationState = { conversationId: 1, expanded: false, turns: [] };
  it("clears planning metadata for direct answers and retains older citations when available", () => {
    let state = conversationReducer(initial, { type: "submit", identity, prompt: "Explain", kind: "initial" });
    const event = { requestId: 7, context: identity };
    state = conversationReducer(state, { type: "research", event, research: { phase: "deciding", query: null, sources: [] } });
    expect(state.turns[0].research?.phase).toBe("deciding");
    state = conversationReducer(state, { type: "research", event, research: { phase: "answering", query: null, sources: [] } });
    expect(state.turns[0].research).toBeUndefined();
    state = conversationReducer(state, { type: "research", event, research: { ...research, phase: "answering" } });
    expect(state.turns[0].research?.sources).toEqual(sources);
  });
  it("ignores stale events and retains sources for follow-ups without another search", () => {
    let state = conversationReducer(initial, { type: "submit", identity, prompt: "Research", kind: "initial" });
    state = conversationReducer(state, { type: "started", event: { requestId: 7, context: identity } });
    expect(conversationReducer(state, { type: "research", event: { requestId: 8, context: identity }, research })).toBe(state);
    state = conversationReducer(state, { type: "research", event: { requestId: 7, context: identity }, research });
    state = conversationReducer(state, { type: "complete", event: { requestId: 7, context: identity } });
    const completed = state;
    expect(conversationReducer(state, { type: "research", event: { requestId: 7, context: identity }, research })).toBe(completed);
    state = conversationReducer(state, { type: "submit", identity: { conversationId: 1, turnId: 2 }, prompt: "Compare", kind: "follow-up" });
    expect(state.turns[1].research?.sources).toEqual(sources);
    expect(state.turns[1].research?.phase).toBe("answering");
    const followUpEvent = { requestId: 8, context: { conversationId: 1, turnId: 2 } };
    state = conversationReducer(state, { type: "research", event: followUpEvent, research: { ...research, sources: [{ ...sources[0], id: 2 }] } });
    state = conversationReducer(state, { type: "error", identity: followUpEvent.context, message: "Search failed" });
    state = conversationReducer(state, { type: "submit", identity: { conversationId: 1, turnId: 3 }, prompt: "Try again", kind: "follow-up", retry: true });
    expect(state.turns[1].research?.sources).toEqual(sources);
    state = conversationReducer(state, { type: "reset", conversationId: 2 });
    expect(conversationReducer(state, { type: "research", event: { requestId: 7, context: identity }, research })).toBe(state);
  });
});
