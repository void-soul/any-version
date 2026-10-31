import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { BuddySwitchProgress, BuddyTransferReport } from "./BuddyPanel.model";
import type { BuddyPlatformScope } from "./platformScope";

export async function runBuddyAccountSwitch({ scope, platform, accountId, onProgress, onResult, onError, onFinished }: {
  scope: BuddyPlatformScope;
  platform: string;
  accountId: string;
  onProgress: (progress: BuddySwitchProgress) => void;
  onResult: (text: string, report: BuddyTransferReport | null) => Promise<void>;
  onError: (error: unknown) => void;
  onFinished: () => void;
}) {
  const lease = scope.capture();
  if (!lease.current()) return;
  let unlisten: UnlistenFn | undefined;
  let detachDisposal: (() => void) | undefined;
  const cleanup = () => { unlisten?.(); unlisten = undefined; detachDisposal?.(); };
  try {
    unlisten = await listen<BuddySwitchProgress>("buddy-switch-progress", event => {
      if (lease.current() && event.payload.platform === platform && event.payload.accountId === accountId) onProgress(event.payload);
    });
    if (!lease.current()) return;
    detachDisposal = scope.onDispose(cleanup);
    const [text, report] = await invoke<[string, BuddyTransferReport | null]>("buddy_switch_account", { platform, accountId });
    if (lease.current()) await onResult(text, report);
  } catch (error) {
    if (lease.current()) onError(error);
  } finally {
    cleanup();
    if (lease.current()) onFinished();
  }
}

export function formatBuddySwitchResult(text: string, report: BuddyTransferReport | null, formatMerge: (report: BuddyTransferReport) => string) {
  const changed = report && report.addedConversations + report.replacedConversations + report.updatedSessionRows + report.scannedWorkspaces > 0;
  return changed ? [text, formatMerge(report)].join("；") : text;
}
