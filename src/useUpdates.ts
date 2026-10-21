import { useEffect, useState, useSyncExternalStore } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { check } from "@tauri-apps/plugin-updater";
import { UpdateController } from "./updates";

function updateStorage() {
  try { return window.localStorage; } catch { return undefined; }
}

// One controller across StrictMode remounts and settings-page changes. Pending
// downloads survive hiding the popup, but never trigger an installation alone.
export const updateController = new UpdateController({
  check: () => check({ timeout: 15_000 }),
  prepareInstall: () => invoke("prepare_update_install"),
  releaseInstall: () => invoke("release_update_install"),
  storage: updateStorage(),
});

export function useUpdates(automatic: boolean) {
  const snapshot = useSyncExternalStore(updateController.subscribe, updateController.getSnapshot);
  const [currentVersion, setCurrentVersion] = useState("");
  useEffect(() => { void getVersion().then(setCurrentVersion).catch(() => undefined); }, []);
  useEffect(() => {
    // Development previews must not install over a user's real application.
    if (!automatic || import.meta.env.DEV) return;
    return updateController.startAutomaticChecks();
  }, [automatic]);
  return { ...snapshot, currentVersion };
}
