import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Snapshot = {
  id: number;
  stage: string;
  capturedAt: string;
  body: unknown;
  note: string | null;
};
type InspectorState = { enabled: boolean; requests: Snapshot[] };

export function RequestInspector({ onLayoutChange }: { onLayoutChange: () => void }) {
  const [state, setState] = useState<InspectorState>({ enabled: false, requests: [] });
  const [expanded, setExpanded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const latestRead = useRef(0);
  const layoutCallback = useRef(onLayoutChange);
  layoutCallback.current = onLayoutChange;

  const refresh = useCallback(async (clear = false) => {
    const read = ++latestRead.current;
    try {
      const next = await invoke<InspectorState>("read_request_inspector", { clear });
      if (read === latestRead.current) {
        setState((previous) => JSON.stringify(previous) === JSON.stringify(next) ? previous : next);
        setError("");
      }
    } catch (error) {
      if (read === latestRead.current) setError(String(error));
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => {
    if (!state.enabled || !expanded) return;
    const timer = window.setInterval(() => void refresh(), 2000);
    return () => window.clearInterval(timer);
  }, [state.enabled, expanded, refresh]);
  useEffect(() => { layoutCallback.current(); }, [state, expanded, error]);

  async function toggle(enabled: boolean) {
    setBusy(true);
    ++latestRead.current;
    try {
      await invoke("configure_request_inspector", { enabled });
      await refresh();
    } catch (error) { setError(String(error)); }
    finally { setBusy(false); }
  }

  return (
    <section className="mt-4 text-xs text-zinc-400">
      <button type="button" aria-expanded={expanded} onClick={() => setExpanded(!expanded)} className="text-[11px] font-medium hover:text-zinc-200">
        {expanded ? "−" : "+"} Model Request Inspector {state.enabled ? "(recording)" : ""}
      </button>
      {expanded && <div className="mt-3 space-y-3">
        <p className="text-[10px] leading-4">Development only. Captures the last 10 model requests in memory, including selected text, conversation, and web evidence. Image data is omitted. Disabling capture clears the history.</p>
        <div className="flex flex-wrap items-center gap-3">
          <label className="flex items-center gap-2"><input type="checkbox" checked={state.enabled} disabled={busy} onChange={(event) => void toggle(event.target.checked)} /> Capture requests</label>
          <button type="button" disabled={busy} onClick={() => void refresh()} className="hover:text-zinc-100">Refresh</button>
          <button type="button" disabled={busy} onClick={() => void refresh(true)} className="hover:text-zinc-100">Clear</button>
        </div>
        {state.requests.length === 0 && <p className="text-[10px]">Enable capture before sending a prompt. Requests appear here as each model stage starts.</p>}
        <div className="preview-scroll max-h-64 space-y-2 overflow-y-auto">
          {[...state.requests].reverse().map((request) => <details key={request.id} onToggle={() => layoutCallback.current()} className="rounded-lg border border-zinc-800 bg-zinc-950/50 p-2">
            <summary className="cursor-pointer break-words text-[10px]">#{request.id} · {request.stage} · {request.capturedAt}</summary>
            {request.note && <p className="mt-2 text-amber-200">{request.note}</p>}
            <pre className="mt-2 whitespace-pre-wrap break-all font-mono text-[10px] leading-4">{JSON.stringify(request.body, null, 2)}</pre>
          </details>)}
        </div>
      </div>}
      {error && <p role="alert" className="mt-2 text-amber-200">{error}</p>}
    </section>
  );
}
