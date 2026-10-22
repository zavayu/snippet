import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ConversationState } from "../conversation";
import { ConversationPanel } from "./ConversationPanel";

function render(conversation: ConversationState, replacementId: number | null = null) {
  const noop = () => {};
  return renderToStaticMarkup(createElement(ConversationPanel, {
    conversation, onContinue: noop, onNewConversation: noop,
    onSubmit: noop, onRetry: noop, onStop: noop, replacementId, onReplacementUsed: noop,
  }));
}

const completed: ConversationState = {
  conversationId: 1, expanded: false,
  turns: [{ id: 1, prompt: "Explain this", response: "First **answer**", status: "complete", kind: "initial" }],
};

describe("conversation interface", () => {
  it("offers replacement only with an authorized selection and a completed response", () => {
    expect(render(completed)).not.toContain('aria-label="Replace selection"');
    expect(render(completed, 42)).toContain('aria-label="Replace selection"');
    expect(render({ ...completed, expanded: true }, 42)).toContain('aria-label="Replace selection"');
    for (const status of ["loading", "streaming", "stopped", "error"] as const) {
      expect(render({ ...completed, turns: [{ ...completed.turns[0], status }] }, 42))
        .not.toContain('aria-label="Replace selection"');
    }
  });

  it("hides replacement on earlier turns while a follow-up is streaming", () => {
    expect(render({ ...completed, expanded: true, turns: [...completed.turns, {
      id: 2, prompt: "Why?", response: "Partial", kind: "follow-up", status: "streaming",
    }] }, 42)).not.toContain('aria-label="Replace selection"');
  });

  it("keeps the first answer compact and offers all three response actions", () => {
    const html = render(completed);
    expect(html).toContain("<strong>answer</strong>");
    expect(html).toContain("Copy response");
    expect(html).toContain("Continue conversation");
    expect(html).toContain("New conversation");
    expect(html).not.toContain("<textarea");
  });

  it("shows both turns, individual copy buttons, and a composer in conversation mode", () => {
    const html = render({ ...completed, expanded: true, turns: [...completed.turns, {
      id: 2, prompt: "Why?", response: "Second answer", kind: "follow-up", status: "complete",
    }] });
    expect(html).toContain("Explain this");
    expect(html).toContain("Why?");
    expect(html).toContain("Second answer");
    expect(html.match(/aria-label="Copy response"/g)).toHaveLength(2);
    expect(html).toContain('aria-label="Follow-up question"');
    expect(html).not.toContain("Continue conversation");
    expect(html).not.toContain('aria-label="New conversation"');
    expect(html.match(/<hr\b/g)).toHaveLength(1);
  });

  it("switches the composer button to an enabled stop control without duplicate actions", () => {
    const html = render({ ...completed, expanded: true, turns: [...completed.turns, {
      id: 2, prompt: "Why?", response: "Partial answer", kind: "follow-up", status: "streaming",
    }] });
    const stopButtons = html.match(/<button\b[^>]*aria-label="Stop response"[^>]*>/g);
    expect(stopButtons).toHaveLength(1);
    expect(stopButtons?.[0]).toContain('type="button"');
    expect(stopButtons?.[0]).not.toContain('disabled=""');
    expect(html).not.toContain('aria-label="New conversation"');
    expect(html).not.toContain('aria-label="Send message"');
    expect(html).not.toContain("Continue conversation");
  });

  it("marks incomplete responses and offers retry instead of treating them as complete", () => {
    const html = render({ ...completed, turns: [{ ...completed.turns[0], status: "stopped" }] });
    expect(html).toContain("Response stopped");
    expect(html).toContain("not included in follow-up context");
    expect(html).toContain("Retry response");
    expect(html).not.toContain("Continue conversation");
  });
});
