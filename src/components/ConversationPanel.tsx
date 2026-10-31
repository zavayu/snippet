import { responseWithSources } from "../research";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { generationLabel, isTurnActive, type ConversationState } from "../conversation";
import { WebLink } from "./WebLink";
import { MarkdownResponse } from "./MarkdownResponse";
import { handlePromptKeyDown } from "../promptKeyboard";

function ResponseAction({ label, onClick, children }: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return <button type="button" aria-label={label} title={label} onClick={onClick}
    className="inline-grid size-7 cursor-pointer place-items-center rounded-md text-zinc-500 transition-colors hover:bg-zinc-800 hover:text-zinc-200 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-violet-400">
    <svg aria-hidden="true" viewBox="0 0 24 24" className="size-4 fill-none stroke-current stroke-[1.8]" strokeLinecap="round" strokeLinejoin="round">
      {children}
    </svg>
  </button>;
}

function CopyResponse({ text }: { text: string }) {
  const [status, setStatus] = useState<"idle" | "copied" | "error">("idle");
  const timeout = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (timeout.current) clearTimeout(timeout.current); }, []);
  const label = status === "copied" ? "Copied response" : status === "error" ? "Copy failed - retry" : "Copy response";
  return <ResponseAction label={label} onClick={async () => {
    try {
      await invoke("copy_response", { text });
      setStatus("copied");
    } catch { setStatus("error"); }
    if (timeout.current) clearTimeout(timeout.current);
    timeout.current = setTimeout(() => setStatus("idle"), 2500);
  }}>
    {status === "copied" ? <path d="m5 12 4.5 4.5L19 7" /> : status === "error" ? <>
      <path d="m12 3 10 18H2L12 3Z" />
      <path d="M12 9v4m0 3h.01" />
    </> : <>
      <rect x="8" y="8" width="11" height="11" rx="2" />
      <path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" />
    </>}
  </ResponseAction>;
}

type Props = {
  conversation: ConversationState;
  onContinue: () => void;
  onNewConversation: () => void;
  onSubmit: (instruction: string) => void;
  onRetry: () => void;
  onStop: () => void;
  replacementId: number | null;
  onReplacementUsed: (id: number) => void;
  onLayoutChange?: () => void;
};

export function ConversationPanel({ conversation, onContinue, onNewConversation, onSubmit, onRetry, onStop, replacementId, onReplacementUsed, onLayoutChange }: Props) {
  const [draft, setDraft] = useState("");
  const [replacing, setReplacing] = useState(false);
  const [replaceError, setReplaceError] = useState<string | null>(null);
  useEffect(() => { setReplaceError(null); }, [conversation.conversationId]);
  const replace = async (text: string) => {
    if (replacementId == null || replacing) return;
    setReplacing(true);
    setReplaceError(null);
    try {
      await invoke("replace_selection", { replacementId, text });
    } catch (error) {
      setReplaceError(String(error));
    } finally {
      onReplacementUsed(replacementId);
      setReplacing(false);
    }
  };
  const replaceAction = (text: string) => replacementId != null && !replacing && <ResponseAction label="Replace selection" onClick={() => void replace(text)}>
    <path d="M4 7h12m-3-3 3 3-3 3M20 17H8m3-3-3 3 3 3" />
  </ResponseAction>;

  const input = useRef<HTMLTextAreaElement>(null);
  const transcript = useRef<HTMLDivElement>(null);
  const followBottom = useRef(true);
  const lastTurnId = useRef<number | undefined>(undefined);
  const last = conversation.turns[conversation.turns.length - 1];
  const busy = isTurnActive(last);
  const hasCompletedTurn = conversation.turns.some((turn) => turn.status === "complete");

  useEffect(() => {
    if (conversation.expanded && !busy) input.current?.focus();
  }, [conversation.expanded, busy]);

  useLayoutEffect(() => {
    const node = transcript.current;
    if (lastTurnId.current !== last?.id) followBottom.current = true;
    if (node && followBottom.current) node.scrollTop = node.scrollHeight;
    lastTurnId.current = last?.id;
  }, [conversation]);

  const submit = () => {
    if (!draft.trim() || busy || !hasCompletedTurn) return;
    onSubmit(draft.trim());
    setDraft("");
    followBottom.current = true;
  };

  return <section className="mb-2" aria-label="Conversation">
    <div ref={transcript} role="log" aria-label="Conversation messages" aria-busy={busy}
      className={`response-scroll conversation-transcript overflow-y-auto text-sm leading-5 text-zinc-200 ${conversation.expanded ? "conversation-transcript--expanded" : ""}`}
      onScroll={(event) => {
        const node = event.currentTarget;
        followBottom.current = node.scrollHeight - node.scrollTop - node.clientHeight < 48;
      }}>
      {conversation.turns.map((turn, index) => <article key={turn.id} className="mb-4 last:mb-0">
        {conversation.expanded && index > 0 && <hr className="mb-4 border-0 border-t border-zinc-800" />}
        {conversation.expanded && <div className="mb-3 ml-auto w-fit max-w-[calc(100%_-_2rem)] rounded-xl bg-zinc-800 px-3 py-2">
          <p className="whitespace-pre-wrap break-words">{turn.prompt}</p>
          {turn.reference && <p className="mt-1 text-xs text-zinc-500">{turn.reference}</p>}
        </div>}
        {conversation.expanded && <p className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-zinc-500">Snippet</p>}
        {turn.research?.phase === "unsupported" && <p className="mt-2 text-xs text-zinc-400">Web tools are unavailable with this model. Choose a tool-capable model in Settings to enable search.</p>}
        {turn.response && <MarkdownResponse content={turn.response} sources={turn.research?.sources} />}
          {!isTurnActive(turn) && turn.research && turn.research.sources.length > 0 && <details className="mt-2 text-xs text-zinc-400" onToggle={onLayoutChange}>
            <summary className="w-fit cursor-pointer rounded py-1 hover:text-zinc-200 focus-visible:outline-2 focus-visible:outline-violet-400">Sources ({turn.research.sources.length}){turn.kind === "follow-up" && turn.research.phase === "answering" ? " from earlier research" : ""}</summary>
            {turn.research.query && <p className="mt-1 break-words">Search: {turn.research.query}</p>}
            <ol className="mt-2 space-y-1 pl-4">
              {turn.research.sources.map((source) => <li key={source.id} className="break-words">
                <WebLink href={source.url}>[{source.id}] {source.title}</WebLink>
              </li>)}
            </ol>
          </details>}
        {conversation.expanded && turn.status === "loading" && <p className="text-xs text-zinc-500">{generationLabel(turn)}</p>}
        {turn.status === "stopped" && <p className="mt-2 text-xs text-zinc-500">Response stopped. This turn is not included in follow-up context.</p>}
        {turn.status === "error" && <p role="alert" className="mt-2 text-xs text-amber-200">{turn.error}{turn.response && " This incomplete turn is not included in follow-up context."}</p>}
        {conversation.expanded && turn.response && !isTurnActive(turn) && <div className="flex items-center gap-1">
          <CopyResponse text={responseWithSources(turn.response, turn.research?.sources)} />
          {!busy && turn.status === "complete" && replaceAction(responseWithSources(turn.response, turn.research?.sources))}
        </div>}
      </article>)}
    </div>

    {(!conversation.expanded || (!busy && (last?.status === "error" || last?.status === "stopped"))) && <div className="mt-3 flex flex-wrap items-center gap-1 border-t border-zinc-800 pt-2">
      {!conversation.expanded && last?.response && !busy && <CopyResponse text={responseWithSources(last.response, last.research?.sources)} />}
      {!conversation.expanded && last?.status === "complete" && replaceAction(responseWithSources(last.response, last.research?.sources))}
      {!conversation.expanded && last?.status === "complete" && <ResponseAction label="Continue conversation" onClick={onContinue}>
        <path d="M8 18H5l-3 3V6a3 3 0 0 1 3-3h14a3 3 0 0 1 3 3v9a3 3 0 0 1-3 3H8Z" />
        <path d="M7 8h10M7 12h7" />
      </ResponseAction>}
      {!busy && (last?.status === "error" || last?.status === "stopped") && <ResponseAction label="Retry response" onClick={onRetry}>
        <path d="M3 10a9 9 0 1 1 2 8M3 4v6h6" />
      </ResponseAction>}
      {!conversation.expanded && busy && <ResponseAction label="Stop response" onClick={onStop}>
        <rect x="5" y="5" width="14" height="14" rx="2" />
      </ResponseAction>}
      {!conversation.expanded && <ResponseAction label="New conversation" onClick={onNewConversation}>
        <rect x="3" y="3" width="18" height="18" rx="3" />
        <path d="M12 8v8m-4-4h8" />
      </ResponseAction>}
    </div>}

    {replaceError && <p role="alert" className="mt-2 text-xs text-amber-200">{replaceError}</p>}

    {conversation.expanded && <form className="mt-2 flex items-center gap-2 rounded-xl border border-zinc-700 bg-zinc-950/50 p-2"
      onSubmit={(event) => { event.preventDefault(); submit(); }}>
      <textarea ref={input} aria-label="Follow-up question" placeholder="Ask a follow-up…" rows={2}
        title="Enter to send; Shift+Enter for a new line"
        className="max-h-32 min-w-0 flex-1 resize-none bg-transparent px-1 py-1 text-sm text-zinc-100 outline-none placeholder:text-zinc-500 disabled:opacity-50"
        value={draft} disabled={!hasCompletedTurn}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => handlePromptKeyDown(event, submit)} />
      <button type={busy ? "button" : "submit"} aria-label={busy ? "Stop response" : "Send message"} title={busy ? "Stop response" : "Send message"}
        className="grid size-8 shrink-0 cursor-pointer place-items-center rounded-full bg-violet-500 text-white transition-colors hover:bg-violet-400 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-violet-400 disabled:cursor-not-allowed disabled:opacity-40"
        disabled={!busy && (!hasCompletedTurn || !draft.trim())}
        onClick={busy ? onStop : undefined}>
        <svg aria-hidden="true" viewBox="0 0 24 24" className="size-4.5 fill-none stroke-current stroke-2" strokeLinecap="round" strokeLinejoin="round">
          {busy ? <rect x="6" y="6" width="12" height="12" rx="1" className="fill-current stroke-none" /> : <path d="M12 19V5m-6 6 6-6 6 6" />}
        </svg>
      </button>
    </form>}
  </section>;
}
