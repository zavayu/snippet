import { openUrl } from "@tauri-apps/plugin-opener";
import { SettingsToggle } from "./SettingsRows";
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
  const buttonStyle = "rounded-lg bg-violet-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-violet-400 disabled:cursor-not-allowed disabled:opacity-50";

  return (
    <section className="space-y-2 text-xs leading-5" aria-label="Application updates">
      <p className="text-zinc-400">Version {update.currentVersion || "…"}</p>
      <SettingsToggle label="Automatic checks" description="Checks for updates daily and lets you choose when to download and install them."
        checked={automatic} onChange={onAutomaticChange} />
      {import.meta.env.DEV && <p className="settings-help">Automatic checks and installation are disabled in development builds.</p>}
      <div role="status" aria-live="polite" className="text-zinc-300">
        {update.phase === "idle" && !update.error && "Check for updates."}
        {update.phase === "checking" && "Checking for updates…"}
        {update.phase === "current" && "You’re up to date."}
        {update.version && ["available", "downloading", "downloaded", "installing"].includes(update.phase) && `Snippet ${update.version} is available.`}
        {update.phase === "downloading" && <p>Downloading… {percent === undefined ? `${(update.downloadedBytes / 1048576).toFixed(1)} MB` : `${percent}%`}</p>}
        {update.phase === "downloaded" && <p>The update is ready to install. Snippet will restart.</p>}
        {update.phase === "installing" && <p>Starting installation…</p>}
      </div>
      {update.phase === "downloading" && <progress aria-label="Update download progress" className="h-2 w-full accent-violet-400" max={100} value={percent} />}
      {update.notes && (
        <div className="rounded-lg border border-zinc-800 p-3">
          <p className="settings-label mb-2">Release notes</p>
          <div className="preview-scroll max-h-40 overflow-y-auto text-zinc-400"><MarkdownResponse content={update.notes} /></div>
        </div>
      )}
      {update.error && <p role="alert" className="text-amber-200">{update.error}</p>}
      {busy && update.phase === "downloaded" && <p className="text-zinc-500">Finish the current request before installing.</p>}
      <div className="flex flex-wrap gap-2">
        {update.phase === "available" && <button type="button" className={buttonStyle} onClick={() => void updateController.download()}>Download update</button>}
        {update.phase === "downloaded" && <button type="button" className={buttonStyle} disabled={busy || import.meta.env.DEV}
          onClick={() => void updateController.install(busy)}>Install and restart</button>}
        {!["downloaded", "installing"].includes(update.phase) && <button type="button" className={buttonStyle} disabled={working}
          onClick={() => void updateController.check()}>Check now</button>}
        <button type="button" className="px-2 text-zinc-400 hover:text-zinc-100"
          onClick={() => void openUrl("https://github.com/zavayu/snippet/releases")}>View releases</button>
      </div>
      {update.lastChecked && <p className="text-zinc-500">Last checked: {new Date(update.lastChecked).toLocaleString()}</p>}
    </section>
  );
}
