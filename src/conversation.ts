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
  | { type: "started" | "complete" | "stopped"; event: GenerationEvent }
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
    return { ...state, turns: [...turns, {
      id: action.identity.turnId, prompt: action.prompt, reference: action.reference,
      kind: action.kind, response: "", status: "loading",
    }] };
  }
  const identity = action.type === "error" ? action.identity : action.event.context;
  if (!identity || identity.conversationId !== state.conversationId || identity.turnId !== last?.id || !isTurnActive(last)) return state;
  let updated = last;
  if (action.type === "error") updated = { ...last, status: "error", error: action.message };
  else if (action.type === "started") {
    if (last.requestId !== undefined && last.requestId !== action.event.requestId) return state;
    updated = { ...last, requestId: action.event.requestId };
  } else {
    if (last.requestId !== action.event.requestId) return state;
    updated = action.type === "chunk"
      ? { ...last, status: "streaming", response: last.response + action.delta }
      : { ...last, status: action.type };
  }
  return { ...state, turns: [...state.turns.slice(0, -1), updated] };
}

export function isTurnActive(turn?: ChatTurn): boolean {
  return turn?.status === "loading" || turn?.status === "streaming";
}
