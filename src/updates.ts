import type { DownloadEvent } from "@tauri-apps/plugin-updater";

export const UPDATE_CHECK_INTERVAL = 24 * 60 * 60 * 1000;
export const UPDATE_STARTUP_DELAY = 10_000;
const LAST_CHECK_KEY = "snippet:last-update-check";

export type UpdatePhase = "idle" | "checking" | "current" | "available" | "downloading" | "downloaded" | "installing";
export type UpdateSnapshot = {
  phase: UpdatePhase;
  version?: string;
  notes?: string;
  downloadedBytes: number;
  totalBytes?: number;
  lastChecked?: number;
  error?: string;
};

// Kept separate from the UI so scheduling, retries, and consent can be tested.
export type UpdatePackage = {
  version: string;
  body?: string;
  download: (onEvent?: (event: DownloadEvent) => void, options?: { timeout?: number }) => Promise<void>;
  install: (options?: { restartAfterInstall?: boolean }) => Promise<void>;
  close: () => Promise<void>;
};

type Dependencies = {
  check: () => Promise<UpdatePackage | null>;
  prepareInstall: () => Promise<unknown>;
  releaseInstall: () => Promise<unknown>;
  now?: () => number;
  storage?: Pick<Storage, "getItem" | "setItem">;
};

export class UpdateController {
  private snapshot: UpdateSnapshot = { phase: "idle", downloadedBytes: 0 };
  private listeners = new Set<() => void>();
  private update: UpdatePackage | null = null;
  private lastAttempt = 0;
  private now: () => number;

  constructor(private dependencies: Dependencies) {
    this.now = dependencies.now ?? Date.now;
    try {
      const saved = Number(dependencies.storage?.getItem(LAST_CHECK_KEY));
      if (Number.isFinite(saved) && saved > 0 && saved <= this.now()) {
        this.lastAttempt = saved;
        this.snapshot.lastChecked = saved;
      }
    } catch { /* Update checking still works if browser storage is unavailable. */ }
  }

  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  private publish(change: Partial<UpdateSnapshot>) {
    this.snapshot = { ...this.snapshot, ...change };
    this.listeners.forEach((listener) => listener());
  }

  startAutomaticChecks() {
    const timer = setTimeout(() => {
      void this.check(false);
    }, UPDATE_STARTUP_DELAY);
    // Recheck eligibility hourly, including after sleep/resume. The timestamp
    // limits actual network requests to once daily across application restarts.
    const interval = setInterval(() => { void this.check(false); }, 60 * 60 * 1000);
    return () => { clearTimeout(timer); clearInterval(interval); };
  }

  async check(manual = true) {
    if (["checking", "downloading", "downloaded", "installing"].includes(this.snapshot.phase)) return;
    const now = this.now();
    if (!manual && this.lastAttempt > 0 && now - this.lastAttempt < UPDATE_CHECK_INTERVAL) return;
    this.lastAttempt = now;
    try { this.dependencies.storage?.setItem(LAST_CHECK_KEY, String(now)); } catch { /* best effort */ }
    this.publish({ phase: "checking", error: undefined, lastChecked: now });
    try {
      const next = await this.dependencies.check();
      const previous = this.update;
      this.update = next;
      if (previous && previous !== next) await previous.close().catch(() => undefined);
      this.publish({
        phase: next ? "available" : "current",
        version: next?.version,
        notes: next?.body,
        downloadedBytes: 0,
        totalBytes: undefined,
      });
    } catch (error) {
      const missingFeed = String(error).includes("404");
      this.publish({
        phase: this.update ? "available" : "idle",
        error: missingFeed
          ? "The update feed is not published yet. Try again later."
          : "Could not check for updates. Check your connection and try again.",
      });
    }
  }

  async download() {
    if (!this.update || this.snapshot.phase !== "available") return;
    this.publish({ phase: "downloading", error: undefined, downloadedBytes: 0, totalBytes: undefined });
    try {
      await this.update.download((event) => {
        if (event.event === "Started") this.publish({ totalBytes: event.data.contentLength });
        if (event.event === "Progress") {
          this.publish({ downloadedBytes: this.snapshot.downloadedBytes + event.data.chunkLength });
        }
      }, { timeout: 120_000 });
      // Tauri verifies the artifact signature before returning downloaded bytes.
      this.publish({ phase: "downloaded" });
    } catch {
      this.publish({ phase: "available", error: "Download or signature verification failed. Nothing was installed. You can retry the download." });
    }
  }

  async install(busy: boolean) {
    if (!this.update || this.snapshot.phase !== "downloaded") return;
    if (busy) {
      this.publish({ error: "Wait for the response or screen reading to finish before installing." });
      return;
    }
    this.publish({ phase: "installing", error: undefined });
    let reserved = false;
    try {
      // Backend checks active captures/generation again and prevents new work.
      await this.dependencies.prepareInstall();
      reserved = true;
      await this.update.install({ restartAfterInstall: true });
      // On Windows, a successful install exits this process; the installer
      // relaunches Snippet with the same app identity and saved configuration.
    } catch {
      if (reserved) await this.dependencies.releaseInstall().catch(() => undefined);
      this.publish({ phase: "downloaded", error: "Could not start installation. Finish any active capture or response and try again." });
    }
  }
}
