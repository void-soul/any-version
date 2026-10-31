import { invoke } from "@tauri-apps/api/core";
import type { BuddyAccount, BuddyAccountView, BuddyPaths } from "./BuddyPanel.model";
import type { BuddyPlatformScope } from "./platformScope";

type AccountSnapshot = { accounts: BuddyAccount[]; currentId: string | null; paths: BuddyPaths; view: BuddyAccountView };

/** The scope is bound when the platform loader is created, never when an old callback resumes. */
export async function loadBuddyPlatform({ scope, platform, onLoaded, onError, onFinished }: {
  scope: BuddyPlatformScope;
  platform: string;
  onLoaded: (snapshot: AccountSnapshot) => void;
  onError: (error: unknown) => void;
  onFinished: () => void;
}) {
  const lease = scope.capture();
  if (!lease.current()) return;
  try {
    const [accounts, currentId, paths, view] = await Promise.all([
      invoke<BuddyAccount[]>("buddy_list_accounts", { platform }),
      invoke<string | null>("buddy_get_current_account_id", { platform }),
      invoke<BuddyPaths>("buddy_get_paths", { platform }),
      invoke<BuddyAccountView>("buddy_get_account_view", { platform }),
    ]);
    if (lease.current()) onLoaded({ accounts, currentId, paths, view });
  } catch (error) {
    if (lease.current()) onError(error);
  } finally {
    if (lease.current()) onFinished();
  }
}
