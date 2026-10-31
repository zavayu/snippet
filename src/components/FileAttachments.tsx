import { useEffect, useRef, useState, type RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";

export type AttachedFile = { fileId: string; name: string; sizeBytes: number; totalLines: number };
export const MAX_FILE_BYTES = 512 * 1024;
export const FILE_ACCEPT = ".txt,.md,.markdown,.rs,.py,.js,.jsx,.ts,.tsx,.json,.jsonl,.yaml,.yml,.toml,.xml,.html,.htm,.css,.scss,.sql,.c,.h,.cpp,.hpp,.cc,.cs,.java,.go,.rb,.php,.swift,.kt,.sh,.bash,.ps1,.r,.csv,.tsv,.log,.ini,.cfg,.conf,.tex,.vue,.svelte,.env,.gitignore";

type Props = {
  inputRef?: RefObject<HTMLInputElement | null>;
  conversationId: number;
  files: AttachedFile[];
  hiddenPreviewIds?: readonly string[];
  disabled: boolean;
  onChange: (files: AttachedFile[]) => void;
  onBusy: (busy: boolean) => void;
  onLayoutChange: () => void;
};

export function useFileAttachments({ inputRef, conversationId, files, hiddenPreviewIds = [], disabled, onChange, onBusy, onLayoutChange }: Props) {
  const ownInput = useRef<HTMLInputElement>(null);
  const input = inputRef ?? ownInput;
  const currentId = useRef(conversationId);
  currentId.current = conversationId;
  const busyRef = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    busyRef.current = false; setBusy(false); setError(null);
    if (input.current) input.current.value = "";
  }, [conversationId]);
  const setWorking = (value: boolean) => { busyRef.current = value; setBusy(value); onBusy(value); };
  const attach = async (selected: File[]) => {
    if (disabled || busyRef.current || !selected.length) return;
    const id = conversationId;
    setError(null);
    if (selected.length + files.length > 5) { setError("Attach up to five files per conversation."); onLayoutChange(); return; }
    setWorking(true);
    try {
      for (const file of selected) {
        if (file.size > MAX_FILE_BYTES) throw new Error(`${file.name}: files must be 512 KB or smaller.`);
        const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
        if (currentId.current !== id) return;
        const updated = await invoke<AttachedFile[]>("attach_file", { conversationId: id, name: file.name, bytes });
        if (currentId.current !== id) return;
        onChange(updated);
      }
    } catch (error) {
      if (currentId.current === id) setError(String(error instanceof Error ? error.message : error));
    } finally {
      if (currentId.current === id) { setWorking(false); onLayoutChange(); }
    }
  };
  const remove = async (fileId: string) => {
    if (disabled || busyRef.current) return;
    const id = conversationId;
    setError(null); setWorking(true);
    try {
      const updated = await invoke<AttachedFile[]>("remove_attached_file", { conversationId: id, fileId });
      if (currentId.current === id) onChange(updated);
    } catch (error) {
      if (currentId.current === id) setError(String(error));
    } finally {
      if (currentId.current === id) { setWorking(false); onLayoutChange(); }
    }
  };
  const picker = <input ref={input} type="file" multiple accept={FILE_ACCEPT} className="hidden" aria-label="Choose text files"
    disabled={disabled || busy}
    onChange={(event) => { const selected = Array.from(event.currentTarget.files ?? []); event.currentTarget.value = ""; void attach(selected); }} />;
  const list = <AttachmentList files={files.filter((file) => !hiddenPreviewIds.includes(file.fileId))} disabled={disabled || busy} error={error} onRemove={(id) => void remove(id)} />;
  return { picker, list };
}

function AttachmentList({ files, disabled, error, onRemove }: {
  files: AttachedFile[]; disabled: boolean; error: string | null; onRemove: (id: string) => void;
}) {
  if (!files.length && !error) return null;
  return <div className="mb-2" aria-label="File attachments">
    <div className="flex max-h-24 flex-wrap gap-2 overflow-y-auto">
      {files.map((file) => <div key={file.fileId}
        className="inline-flex max-w-full items-center gap-2 rounded-lg border border-zinc-700/70 bg-zinc-800/60 py-1 pl-2 pr-1 text-xs text-zinc-300">
        <svg aria-hidden="true" viewBox="0 0 24 24" className="size-4 shrink-0 fill-none stroke-current stroke-[1.6] text-zinc-500" strokeLinecap="round" strokeLinejoin="round">
          <path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9l-6-6Z" /><path d="M14 3v6h6M8 13h8M8 17h5" />
        </svg>
        <span className="min-w-0 max-w-48 truncate" title={`${file.name} - ${file.totalLines} lines - ${Math.ceil(file.sizeBytes / 1024)} KB`}>{file.name}</span>
        <button type="button" disabled={disabled} aria-label={`Remove ${file.name}`} title="Remove attachment; previously read excerpts remain in the conversation"
          className="grid size-6 shrink-0 cursor-pointer place-items-center rounded text-zinc-500 transition-colors hover:bg-zinc-700 hover:text-zinc-200 focus-visible:outline-2 focus-visible:outline-violet-400 disabled:cursor-not-allowed disabled:opacity-40" onClick={() => onRemove(file.fileId)}>
          <svg aria-hidden="true" viewBox="0 0 24 24" className="size-3.5 fill-none stroke-current stroke-2" strokeLinecap="round"><path d="m7 7 10 10M17 7 7 17" /></svg>
        </button>
      </div>)}
    </div>
    {error && <p role="alert" className="mt-1 text-xs text-amber-200">{error}</p>}
  </div>;
}

export function FileAttachments(props: Props) {
  const { picker, list } = useFileAttachments(props);
  return <>{picker}{list}</>;
}

export function AttachFilesButton({ disabled, busy, onClick }: { disabled: boolean; busy: boolean; onClick: () => void }) {
  return <button type="button" aria-label="Attach files"
    title={busy ? "Updating files..." : "Add files"}
    disabled={disabled} onClick={onClick}
    className="grid size-8 shrink-0 self-start cursor-pointer place-items-center rounded-full text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-zinc-200 focus-visible:outline-2 focus-visible:outline-violet-400 disabled:cursor-not-allowed disabled:opacity-40">
    <svg aria-hidden="true" viewBox="0 0 24 24" className={`size-4.5 fill-none stroke-current stroke-[1.8] ${busy ? "animate-spin" : ""}`} strokeLinecap="round">
      {busy ? <path d="M20 12a8 8 0 1 0-8 8" /> : <path d="M12 5v14M5 12h14" />}
    </svg>
  </button>;
}
