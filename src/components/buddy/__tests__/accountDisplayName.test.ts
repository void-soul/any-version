import { describe, expect, it } from "vitest";
import type { BuddyAccount } from "../BuddyPanel.model";
import { buddyAccountDisplayName } from "../accountDisplayName";
import { filterBuddyAccounts } from "../accountSearch";

const account = { id: "one", email: "a@example.com", nickname: "138****1234", uid: "uid" } as BuddyAccount;

describe("Buddy account remarks", () => {
  it("prefers a trimmed remark without changing the original identity fields", () => {
    const value = { ...account, remark: "  工作账号  " };
    expect(buddyAccountDisplayName(value)).toBe("工作账号");
    expect(value.nickname).toBe("138****1234");
    expect(value.email).toBe("a@example.com");
  });
  it("restores the original fallback chain when the remark is cleared", () => {
    expect(buddyAccountDisplayName({ ...account, remark: "  " })).toBe("138****1234");
    expect(buddyAccountDisplayName({ ...account, remark: null, nickname: null })).toBe("a@example.com");
    expect(buddyAccountDisplayName({ ...account, nickname: "", email: "" })).toBe("uid");
    expect(buddyAccountDisplayName({ ...account, nickname: "", email: "", uid: null })).toBe("unknown");
  });
  it("finds remarks and original masked names while retaining the credential exclusion", () => {
    const value = { ...account, remark: "团队ALPHA", accessToken: "secret-access" };
    expect(filterBuddyAccounts([value], "alpha")).toEqual([value]);
    expect(filterBuddyAccounts([value], "138")).toEqual([value]);
    expect(filterBuddyAccounts([value], "secret-access")).toEqual([]);
  });
  it("displays and finds a full phone number only when manually saved as a remark", () => {
    const value = { ...account, remark: "13800001234" };
    expect(buddyAccountDisplayName(value)).toBe("13800001234");
    expect(filterBuddyAccounts([value], "13800001234")).toEqual([value]);
    expect(filterBuddyAccounts([account], "13800001234")).toEqual([]);
  });
});
