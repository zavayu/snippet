import { afterEach, describe, expect, it, vi } from "vitest";
import { UPDATE_CHECK_INTERVAL, UPDATE_STARTUP_DELAY, UpdateController, type UpdatePackage } from "./updates";

function fixture() {
  let clock = 1_800_000_000_000;
  const storage = new Map<string, string>();
  const update = {
    version: "0.3.0", body: "Fixes and improvements",
    download: vi.fn<UpdatePackage["download"]>().mockResolvedValue(undefined),
    install: vi.fn<UpdatePackage["install"]>().mockResolvedValue(undefined),
    close: vi.fn<UpdatePackage["close"]>().mockResolvedValue(undefined),
  };
  const dependencies = {
    check: vi.fn<() => Promise<UpdatePackage | null>>().mockResolvedValue(update),
    prepareInstall: vi.fn().mockResolvedValue(undefined),
    releaseInstall: vi.fn().mockResolvedValue(undefined),
    now: () => clock,
    storage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => { storage.set(key, value); } },
  };
  return { update, dependencies, controller: new UpdateController(dependencies), advance: (ms: number) => { clock += ms; } };
}

afterEach(() => vi.useRealTimers());

describe("update consent and verification failures", () => {
  it("checking discovers notes without downloading or installing", async () => {
    const { controller, update } = fixture();
    await controller.check();
    expect(controller.getSnapshot()).toMatchObject({ phase: "available", version: "0.3.0", notes: update.body });
    expect(update.download).not.toHaveBeenCalled();
    expect(update.install).not.toHaveBeenCalled();
  });

  it("downloading reports progress and still requires an explicit install", async () => {
    const { controller, update } = fixture();
    update.download.mockImplementation(async (onEvent) => {
      onEvent?.({ event: "Started", data: { contentLength: 100 } });
      onEvent?.({ event: "Progress", data: { chunkLength: 40 } });
      onEvent?.({ event: "Progress", data: { chunkLength: 60 } });
    });
    await controller.check();
    await controller.download();
    expect(controller.getSnapshot()).toMatchObject({ phase: "downloaded", downloadedBytes: 100, totalBytes: 100 });
    expect(update.install).not.toHaveBeenCalled();
    await controller.install(false);
    expect(update.install).toHaveBeenCalledWith({ restartAfterInstall: true });
  });

  it("a bad signature leaves no installable package and permits a retry", async () => {
    const { controller, update } = fixture();
    update.download.mockRejectedValueOnce(new Error("signature mismatch"));
    await controller.check();
    await controller.download();
    await controller.install(false);
    expect(controller.getSnapshot()).toMatchObject({ phase: "available", error: expect.stringContaining("signature") });
    expect(update.install).not.toHaveBeenCalled();
    await controller.download();
    expect(controller.getSnapshot().phase).toBe("downloaded");
  });

  it("does not install during generation and checks backend activity again", async () => {
    const { controller, update, dependencies } = fixture();
    await controller.check();
    await controller.download();
    await controller.install(true);
    expect(dependencies.prepareInstall).not.toHaveBeenCalled();
    dependencies.prepareInstall.mockRejectedValueOnce(new Error("capture active"));
    await controller.install(false);
    expect(update.install).not.toHaveBeenCalled();
    expect(dependencies.releaseInstall).not.toHaveBeenCalled();
    expect(controller.getSnapshot().phase).toBe("downloaded");
  });

  it("releases the backend guard after a failed installer launch", async () => {
    const { controller, update, dependencies } = fixture();
    update.install.mockRejectedValueOnce(new Error("installer launch failed"));
    await controller.check();
    await controller.download();
    await controller.install(false);
    expect(dependencies.releaseInstall).toHaveBeenCalledOnce();
    expect(controller.getSnapshot().phase).toBe("downloaded");
    await controller.install(false);
    expect(update.install).toHaveBeenCalledTimes(2);
  });

  it("coalesces overlapping checks and closes superseded update resources", async () => {
    const { controller, update, dependencies } = fixture();
    let resolve!: (value: UpdatePackage) => void;
    dependencies.check.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
    const first = controller.check();
    await controller.check();
    expect(dependencies.check).toHaveBeenCalledOnce();
    resolve(update);
    await first;
    dependencies.check.mockResolvedValueOnce(null);
    await controller.check();
    expect(update.close).toHaveBeenCalledOnce();
    expect(controller.getSnapshot().phase).toBe("current");
  });
});

describe("daily background checks", () => {
  it("waits after startup and does not check more than once a day", async () => {
    vi.useFakeTimers();
    const { controller, dependencies, advance } = fixture();
    const stop = controller.startAutomaticChecks();
    await vi.advanceTimersByTimeAsync(UPDATE_STARTUP_DELAY - 1);
    expect(dependencies.check).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(dependencies.check).toHaveBeenCalledOnce();
    advance(UPDATE_CHECK_INTERVAL - 1);
    await controller.check(false);
    expect(dependencies.check).toHaveBeenCalledOnce();
    advance(1);
    await controller.check(false);
    expect(dependencies.check).toHaveBeenCalledTimes(2);
    stop();
  });

  it("keeps the daily limit across restarts, including network failures", async () => {
    const { controller, dependencies, advance } = fixture();
    dependencies.check.mockRejectedValueOnce(new Error("offline"));
    await controller.check(false);
    const restarted = new UpdateController(dependencies);
    await restarted.check(false);
    expect(dependencies.check).toHaveBeenCalledOnce();
    advance(UPDATE_CHECK_INTERVAL);
    await restarted.check(false);
    expect(dependencies.check).toHaveBeenCalledTimes(2);
  });

  it("allows a manual retry before the next daily check", async () => {
    const { controller, dependencies } = fixture();
    dependencies.check.mockRejectedValueOnce(new Error("HTTP 404"));
    await controller.check(false);
    expect(controller.getSnapshot().error).toContain("not published");
    await controller.check();
    expect(controller.getSnapshot().phase).toBe("available");
  });

  it("cleans up timers when automatic checks are disabled", async () => {
    vi.useFakeTimers();
    const { controller, dependencies } = fixture();
    controller.startAutomaticChecks()();
    await vi.advanceTimersByTimeAsync(UPDATE_CHECK_INTERVAL * 2);
    expect(dependencies.check).not.toHaveBeenCalled();
  });

  it("does not replace a downloaded package with a background check", async () => {
    const { controller, dependencies, advance } = fixture();
    await controller.check();
    await controller.download();
    advance(UPDATE_CHECK_INTERVAL);
    await controller.check(false);
    expect(dependencies.check).toHaveBeenCalledOnce();
    expect(controller.getSnapshot().phase).toBe("downloaded");
  });
});
