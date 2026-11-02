import type { BuddyAccount } from "./BuddyPanel.model";

/** Search display/identity fields only; credentials and raw responses are excluded. */
export function filterBuddyAccounts(accounts: BuddyAccount[], query: string): BuddyAccount[] {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return accounts;
  return accounts.filter((account) => {
    const fields = [account.nickname, account.email, account.uid, account.id,
      account.enterpriseName, ...(account.tags ?? [])].filter(Boolean).map((value) => value!.toLowerCase());
    return terms.every((term) => fields.some((value) => value.includes(term)));
  });
}

export function toggleSearchResultSelection(accounts: BuddyAccount[], selected: Set<string>): Set<string> {
  return accounts.length > 0 && accounts.every((account) => selected.has(account.id))
    ? new Set() : new Set(accounts.map((account) => account.id));
}
