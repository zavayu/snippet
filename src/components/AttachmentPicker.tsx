import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export type CapturedContextImage = {
  imageId: number;
  source: "screen" | "region" | "window";
  title?: string;
  preview: { dataUrl: string; width: number; height: number };
};
type WindowChoice = { id: number; pid: number; title: string; appName: string; minimized: boolean; iconDataUrl?: string | null };

export function WindowAppIcon({ source }: { source?: string | null }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [source]);
  if (source && !failed) return <img src={source} alt="" aria-hidden="true" className="size-5 shrink-0 object-contain" onError={() => setFailed(true)} />;
  return <svg aria-hidden="true" viewBox="0 0 24 24" className="size-5 shrink-0 fill-none stroke-zinc-500 stroke-[1.5]"><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M3 9h18" /></svg>;
}

export function AttachmentPicker({ conversationId, filesDisabled, onFile, onCapture, onClose, onBusy, onLayoutChange }: {
  conversationId: number; filesDisabled: boolean;
  onFile: () => void; onCapture: (image: CapturedContextImage) => void; onClose: () => void;
  onBusy: (busy: boolean) => void; onLayoutChange: () => void;
}) {
  const [page, setPage] = useState<"menu" | "windows">("menu");
  const [windows, setWindows] = useState<WindowChoice[]>([]);
  const [loading, setLoading] = useState(false);
  const [capturing, setCapturing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const root = useRef<HTMLDivElement>(null);
  const live = useRef(true);
  const request = useRef(0);
  const busy = useRef(false);
  useEffect(() => {
    live.current = true;
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); if (!busy.current) onClose(); }
    };
    const outside = (event: PointerEvent) => {
      if (!busy.current && event.target instanceof Element && !root.current?.contains(event.target)
        && !event.target.closest('[aria-label="Add context"]')) onClose();
    };
    document.addEventListener("keydown", escape, true);
    document.addEventListener("pointerdown", outside);
    return () => {
      live.current = false;
      if (busy.current) onBusy(false);
      document.removeEventListener("keydown", escape, true);
      document.removeEventListener("pointerdown", outside);
    };
  }, []);
  useEffect(() => {
    const target = page === "menu"
      ? root.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")
      : root.current?.querySelector<HTMLInputElement>("input");
    target?.focus({ preventScroll: true });
  }, [page]);
  useEffect(onLayoutChange, [page, windows, loading, capturing, error, query]);

  const refresh = async () => {
    const revision = ++request.current;
    setLoading(true); setError(null);
    try {
      const choices = await invoke<WindowChoice[]>("list_context_windows");
      if (live.current && revision === request.current) setWindows(choices);
    } catch (error) {
      if (live.current && revision === request.current) setError(String(error));
    } finally {
      if (live.current && revision === request.current) setLoading(false);
    }
  };
  const capture = async (window: WindowChoice) => {
    if (busy.current) return;
    busy.current = true; setCapturing(true); setError(null); onBusy(true);
    try {
      const image = await invoke<CapturedContextImage>("capture_context_window", {
        conversationId, windowId: window.id, pid: window.pid,
      });
      if (live.current) { onCapture(image); onClose(); }
    } catch (error) { if (live.current) setError(String(error)); }
    finally {
      busy.current = false;
      if (live.current) { setCapturing(false); onBusy(false); }
    }
  };
  const button = "flex w-full cursor-pointer items-center gap-3 rounded-lg px-3 py-2.5 text-left text-[13px] text-zinc-200 transition-colors hover:bg-zinc-700/70 focus-visible:outline-2 focus-visible:outline-violet-400 disabled:cursor-not-allowed disabled:opacity-40";
  const icon = "size-4 shrink-0 fill-none stroke-current stroke-[1.7] text-zinc-400";
  const filtered = windows.filter((window) => `${window.title} ${window.appName}`.toLowerCase().includes(query.toLowerCase()));
  return <div ref={root} role="dialog" className={`attachment-menu preview-scroll rounded-2xl bg-zinc-900 p-1.5 ${page === "menu" ? "w-60" : "w-[360px]"}`} aria-label="Add context options">
    {page === "menu" ? <>
      <button type="button" className={button} disabled={filesDisabled} onClick={() => { onFile(); onClose(); }}>
        <svg aria-hidden="true" viewBox="0 0 24 24" className={icon} strokeLinecap="round" strokeLinejoin="round"><path d="m8 13 7-7a3 3 0 0 1 4 4l-9 9a5 5 0 0 1-7-7l9-9" /><path d="m7 14 8-8" /></svg>
        Add file
      </button>
      <button type="button" className={button} onClick={() => { setPage("windows"); void refresh(); }}>
        <svg aria-hidden="true" viewBox="0 0 24 24" className={icon} strokeLinecap="round" strokeLinejoin="round"><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M3 9h18M7 6.5h.01m3 0h.01" /></svg>
        <span className="flex-1">Add window</span>
        <svg aria-hidden="true" viewBox="0 0 24 24" className="size-3.5 fill-none stroke-zinc-500 stroke-[1.7]"><path d="m9 5 7 7-7 7" /></svg>
      </button>
    </> : <>
      <div className="mb-2 flex items-center gap-2 px-1 pt-1 text-xs text-zinc-300">
        <button type="button" aria-label="Back to attachment options" disabled={capturing} className="grid size-7 cursor-pointer place-items-center rounded-lg hover:bg-zinc-700 disabled:opacity-40" onClick={() => setPage("menu")}>
          <svg aria-hidden="true" viewBox="0 0 24 24" className={icon} strokeLinecap="round" strokeLinejoin="round"><path d="m14 6-6 6 6 6" /></svg>
        </button>
        <span className="flex-1">Choose a window</span>
        <button type="button" aria-label="Close attachment picker" disabled={capturing} className="grid size-7 cursor-pointer place-items-center rounded-lg hover:bg-zinc-700 disabled:opacity-40" onClick={onClose}>
          <svg aria-hidden="true" viewBox="0 0 24 24" className={icon} strokeLinecap="round"><path d="m7 7 10 10M7 17 17 7" /></svg>
        </button>
      </div>
      <div className="mb-2 flex gap-2">
        <input aria-label="Search windows" placeholder="Search windows…" value={query} disabled={capturing}
          onChange={(event) => setQuery(event.target.value)} className="min-w-0 flex-1 rounded-lg border border-zinc-800 bg-zinc-900 px-2 py-1.5 text-xs text-zinc-200 outline-none focus:border-zinc-600" />
        <button type="button" aria-label="Refresh windows" title="Refresh windows" className="grid size-8 cursor-pointer place-items-center rounded-lg text-zinc-400 hover:bg-zinc-700 disabled:opacity-40" disabled={loading || capturing} onClick={() => void refresh()}>
          <svg aria-hidden="true" viewBox="0 0 24 24" className={`${icon} ${loading ? "animate-spin" : ""}`} strokeLinecap="round" strokeLinejoin="round"><path d="M20 7v5h-5M4 17v-5h5M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1" /></svg>
        </button>
      </div>
      <div className="preview-scroll max-h-48 overflow-y-auto" aria-label="Open windows" aria-busy={loading || capturing}>
        {loading ? <p className="p-2 text-xs text-zinc-500">Finding windows…</p> : filtered.map((window) =>
          <button type="button" key={`${window.id}-${window.pid}`} className={`${button} flex items-center gap-2`} disabled={capturing} onClick={() => void capture(window)}>
            <WindowAppIcon source={window.iconDataUrl} />
            <span className="min-w-0 flex-1"><span className="block truncate" title={window.title}>{window.title}</span><span className="block truncate text-[11px] text-zinc-500">{window.appName || "Window"}{window.minimized ? " · Minimized" : ""}</span></span>
          </button>)}
        {!loading && !windows.length && <p className="p-2 text-xs text-zinc-500">No available windows. Open an app and refresh.</p>}
        {!loading && windows.length > 0 && !filtered.length && <p className="p-2 text-xs text-zinc-500">No windows match your search.</p>}
      </div>
      {capturing && <p role="status" className="mt-2 px-1 text-xs text-violet-300">Capturing window…</p>}
    </>}
    {error && <p role="alert" className="mt-2 break-words px-1 text-xs text-warning">{error}</p>}
  </div>;
}

export function ImageAttachmentPreview({ image, disabled, onRemove, onLayoutChange }: {
  image: CapturedContextImage; disabled: boolean; onRemove: () => void; onLayoutChange?: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  useEffect(() => { setExpanded(false); }, [image.imageId]);
  useEffect(() => { onLayoutChange?.(); }, [expanded]);
  const label = image.source === "window" ? image.title ?? "Window capture" : image.source === "region" ? "Region capture" : "Screen capture";
  return <div className="mb-3 rounded-lg border border-zinc-800 bg-zinc-950/50 p-2" aria-label="Image attachment">
    <div className="flex min-w-0 items-center gap-2">
    <button type="button" aria-label="Preview screenshot" aria-expanded={expanded} title="Preview screenshot" onClick={() => setExpanded((value) => !value)} className="shrink-0 cursor-pointer rounded focus-visible:outline-2 focus-visible:outline-violet-400">
      <img alt={label} className="size-12 rounded object-contain" src={image.preview.dataUrl} />
    </button>
    <span className="min-w-0 flex-1"><span className="block truncate text-xs text-zinc-300" title={label}>{label}</span><span className="block text-[10px] text-zinc-500">Screenshot · {image.preview.width} × {image.preview.height}</span></span>
    <button type="button" disabled={disabled} aria-label="Remove captured image" className="grid size-6 shrink-0 cursor-pointer place-items-center rounded text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200 disabled:opacity-40" onClick={onRemove}>×</button>
    </div>
    {expanded && <img alt={`Screenshot of ${label}`} src={image.preview.dataUrl} className="mt-2 max-h-48 w-full rounded object-contain" onLoad={onLayoutChange} />}
  </div>;
}
