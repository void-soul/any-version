import type { BuddyAccount } from "./BuddyPanel.model";

export function buddyAccountDisplayName(account: BuddyAccount): string {
  return account.remark?.trim() || account.nickname?.trim() || account.email || account.uid || "unknown";
}
