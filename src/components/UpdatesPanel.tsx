import { openUrl } from "@tauri-apps/plugin-opener";
import { MarkdownResponse } from "./MarkdownResponse";
import type { UpdateSnapshot } from "../updates";
import { updateController } from "../useUpdates";

type Props = {
  update: UpdateSnapshot & { currentVersion: string };
  automatic: boolean;
  onAutomaticChange: (enabled: boolean) => void;
  busy: boolean;
};

export function UpdatesPanel({ update, automatic, onAutomaticChange, busy }: Props) {
  const working = ["checking", "downloading", "installing"].includes(update.phase);
  const percent = update.totalBytes ? Math.min(100, Math.round(update.downloadedBytes / update.totalBytes * 100)) : undefined;
  const buttonStyle = "rounded-lg bg-violet-500 px-3 py-2 text-xs font-semibold text-white hover:bg-violet-400 disabled:cursor-not-allowed disabled:opacity-50";

  return (
    <section className="space-y-3 text-xs leading-5" aria-label="Application updates">
      <p className="text-zinc-400">Installed version: {update.currentVersion || "…"}</p>
      <label className="flex items-center justify-between gap-3 rounded-lg border border-zinc-800 px-3 py-2">
        <span>
          <span className="block text-sm text-zinc-200">Automatically check for updates</span>
          <span className="text-zinc-500">Check daily. Downloads and installation are your choice.</span>
        </span>
        <input type="checkbox" className="size-3.5 accent-violet-400" checked={automatic}
          onChange={(event) => onAutomaticChange(event.target.checked)} />
      </label>
      {import.meta.env.DEV && <p className="text-zinc-500">Automatic checks and installation are disabled in development. Test installation with an installed build.</p>}
      <div role="status" aria-live="polite" className="text-zinc-300">
        {update.phase === "idle" && !update.error && "Check for a newer version of Snippet."}
        {update.phase === "checking" && "Checking for updates…"}
        {update.phase === "current" && "You’re up to date."}
        {update.version && ["available", "downloading", "downloaded", "installing"].includes(update.phase) && `Snippet ${update.version} is available.`}
        {update.phase === "downloading" && <p>Downloading… {percent === undefined ? `${(update.downloadedBytes / 1048576).toFixed(1)} MB` : `${percent}%`}</p>}
        {update.phase === "downloaded" && <p>Download verified. Snippet will close briefly and restart during installation.</p>}
        {update.phase === "installing" && <p>Starting installation…</p>}
      </div>
      {update.phase === "downloading" && <progress aria-label="Update download progress" className="h-2 w-full accent-violet-400" max={100} value={percent} />}
      {update.notes && (
        <div className="rounded-lg border border-zinc-800 p-3">
          <p className="mb-2 font-semibold text-zinc-200">Release notes</p>
          <div className="preview-scroll max-h-40 overflow-y-auto text-zinc-400"><MarkdownResponse content={update.notes} /></div>
        </div>
      )}
      {update.error && <p role="alert" className="text-amber-200">{update.error}</p>}
      {busy && update.phase === "downloaded" && <p className="text-zinc-500">Finish the current response or screen reading before installing.</p>}
      <div className="flex flex-wrap gap-2">
        {update.phase === "available" && <button type="button" className={buttonStyle} onClick={() => void updateController.download()}>Download update</button>}
        {update.phase === "downloaded" && <button type="button" className={buttonStyle} disabled={busy || import.meta.env.DEV}
          onClick={() => void updateController.install(busy)}>Install and restart</button>}
        {!["downloaded", "installing"].includes(update.phase) && <button type="button" className={buttonStyle} disabled={working}
          onClick={() => void updateController.check()}>Check now</button>}
        <button type="button" className="px-2 text-zinc-400 hover:text-zinc-100"
          onClick={() => void openUrl("https://github.com/zavayu/snippet/releases")}>View releases</button>
      </div>
      {update.lastChecked && <p className="text-zinc-500">Last check attempted: {new Date(update.lastChecked).toLocaleString()}</p>}
    </section>
  );
}
