import { invoke } from "@tauri-apps/api/core";
import type { AppSettings, AppSettingsPatch } from "../contracts/appSettings";

// Keep requests ordered within this renderer. Each patch still merges in a backend
// transaction, which also protects updates from other windows and native callers.
let pending: Promise<unknown> = Promise.resolve();

function ordered<T>(request: () => Promise<T>): Promise<T> {
  const result = pending.then(request, request);
  pending = result.then(() => undefined, () => undefined);
  return result;
}

export function loadAppSettings(): Promise<AppSettings> {
  return ordered(() => invoke<AppSettings>("launcher_get_settings"));
}

/** Submit only fields owned by this action; never send a cached settings object. */
export function patchAppSettings(patch: AppSettingsPatch): Promise<AppSettings> {
  return ordered(() => invoke<AppSettings>("launcher_patch_settings", { patch }));
}
