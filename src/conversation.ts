import type { ResearchInfo } from "./research";
export type TurnIdentity = { conversationId: number; turnId: number };
export type GenerationEvent = {
  requestId: number;
  context?: TurnIdentity;
};
export type ChatTurn = {
  id: number;
  prompt: string;
  reference?: string;
  kind: "initial" | "follow-up";
  response: string;
  status: "loading" | "streaming" | "complete" | "stopped" | "error";
  requestId?: number;
  error?: string;
  research?: ResearchInfo;
  loadingModel?: boolean;
};
export type ConversationState = {
  conversationId: number;
  expanded: boolean;
  turns: ChatTurn[];
};
export type ConversationAction =
  | { type: "reset"; conversationId: number }
  | { type: "expand" }
  | { type: "submit"; identity: TurnIdentity; prompt: string; reference?: string; kind: ChatTurn["kind"]; retry?: boolean }
  | { type: "started" | "complete" | "stopped" | "content-reset"; event: GenerationEvent }
  | { type: "research"; event: GenerationEvent; research: ResearchInfo }
  | { type: "model-loading"; event: GenerationEvent; loadingModel: boolean }
  | { type: "chunk"; event: GenerationEvent; delta: string }
  | { type: "error"; identity: TurnIdentity; message: string };

export function conversationReducer(state: ConversationState, action: ConversationAction): ConversationState {
  if (action.type === "reset") {
    return { conversationId: action.conversationId, expanded: false, turns: [] };
  }
  if (action.type === "expand") return { ...state, expanded: true };
  const last = state.turns[state.turns.length - 1];
  if (action.type === "submit") {
    if (action.identity.conversationId !== state.conversationId || (last && isTurnActive(last))) return state;
    const turns = action.retry ? state.turns.slice(0, -1) : state.turns;
    const retainedResearch = [...turns].reverse().find((turn) => turn.status === "complete")?.research;
    return { ...state, turns: [...turns, {
      id: action.identity.turnId, prompt: action.prompt, reference: action.reference,
      kind: action.kind, response: "", status: "loading",
      research: action.kind === "follow-up" && retainedResearch ? { ...retainedResearch, phase: "answering" } : undefined,
    }] };
  }
  const identity = action.type === "error" ? action.identity : action.event.context;
  if (!identity || identity.conversationId !== state.conversationId || identity.turnId !== last?.id || !isTurnActive(last)) return state;
  let updated = last;
  if (action.type === "error") updated = { ...last, status: "error", error: action.message, loadingModel: false };
  else if (action.type === "model-loading") {
    if (last.status !== "loading" || (last.requestId !== undefined && last.requestId !== action.event.requestId)) return state;
    updated = { ...last, requestId: action.event.requestId, loadingModel: action.loadingModel };
  } else if (action.type === "research") {
    if (last.requestId !== undefined && last.requestId !== action.event.requestId) return state;
    updated = { ...last, requestId: action.event.requestId,
      research: action.research.phase === "answering" && !action.research.sources.length ? undefined : action.research };
  } else if (action.type === "started") {
    if (last.requestId !== undefined && last.requestId !== action.event.requestId) return state;
    updated = { ...last, requestId: action.event.requestId };
  } else {
    if (last.requestId !== action.event.requestId) return state;
    if (action.type === "content-reset") updated = { ...last, response: "", status: "loading" };
    else updated = action.type === "chunk"
      ? { ...last, status: "streaming", response: last.response + action.delta, loadingModel: false }
      : { ...last, status: action.type, loadingModel: false };
  }
  return { ...state, turns: [...state.turns.slice(0, -1), updated] };
}

export function isTurnActive(turn?: ChatTurn): boolean {
  return turn?.status === "loading" || turn?.status === "streaming";
}

export function generationLabel(turn?: ChatTurn): string {
  if (turn?.status === "streaming") return "Responding...";
  if (turn?.loadingModel) return "Loading model...";
  switch (turn?.research?.phase) {
    case "planning": return "Preparing search...";
    case "searching": return "Searching...";
    case "summarizing": return "Summarizing...";
    default: return "Thinking...";
  }
}
