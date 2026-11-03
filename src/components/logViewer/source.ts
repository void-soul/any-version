import { invoke, isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export type LogSource = { getFile(): Promise<File> };
export type LogSelection = { source: LogSource | null; file: File | null };
type PickerWindow = Window & { showOpenFilePicker?: (options: unknown) => Promise<LogSource[]> };

/** A File from an input/drop is a snapshot; only a handle/path can be refreshed. */
export async function pickLogSource(): Promise<LogSource | null | undefined> {
  if (isTauri()) {
    const path = await open({ multiple: false, filters: [{ name: "Log", extensions: ["log", "txt", "out"] }] });
    if (typeof path !== "string") return null;
    return { async getFile() {
      const buffer = await invoke<ArrayBuffer>("read_log_file", { path });
      return new File([buffer], path.split(/[\\/]/).pop() || "log");
    } };
  }
  const picker = (window as PickerWindow).showOpenFilePicker;
  if (!picker) return undefined;
  const handles = await picker.call(window, { multiple: false });
  return handles[0] || null;
}

export function sameLogBytes(previous: Uint8Array | null, next: Uint8Array): boolean {
  return previous !== null && previous.length === next.length && previous.every((value, index) => value === next[index]);
}
