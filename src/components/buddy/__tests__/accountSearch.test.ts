import { describe, expect, it } from "vitest";
import type { BuddyAccount } from "../BuddyPanel.model";
import { filterBuddyAccounts, toggleSearchResultSelection } from "../accountSearch";

const accounts = [
  { id: "one", nickname: "开发账号", email: "Alice@example.com", uid: "UID-123", tags: ["主力"], accessToken: "secret-access" },
  { id: "two", nickname: "138****5678", email: "", uid: "UID-456", enterpriseName: "测试团队", tags: ["备用"], refreshToken: "secret-refresh" },
] as BuddyAccount[];

describe("Buddy account search", () => {
  it("returns the sorted input unchanged for an empty query", () => {
    expect(filterBuddyAccounts(accounts, "  ")).toBe(accounts);
  });
  it("matches nickname, email, UID, tags, organization and masked phone text", () => {
    for (const query of ["开发", "ALICE@EXAMPLE", "uid-123", "主力", "one"])
      expect(filterBuddyAccounts(accounts, query).map((account) => account.id)).toEqual(["one"]);
    for (const query of ["138", "5678", "****", "备用", "测试团队"])
      expect(filterBuddyAccounts(accounts, query).map((account) => account.id)).toEqual(["two"]);
  });
  it("requires each whitespace-separated term and preserves input order", () => {
    expect(filterBuddyAccounts(accounts, "ALICE 主力")).toEqual([accounts[0]]);
    expect(filterBuddyAccounts(accounts, "Alice 备用")).toEqual([]);
    expect(filterBuddyAccounts([...accounts].reverse(), "uid")).toEqual([...accounts].reverse());
  });
  it("does not search credentials or infer hidden phone digits", () => {
    for (const query of ["secret-access", "secret-refresh", "13800005678", "[.*]"])
      expect(filterBuddyAccounts(accounts, query)).toEqual([]);
  });
  it("selects only visible matches and clears when those matches are all selected", () => {
    expect([...toggleSearchResultSelection([accounts[1]], new Set(["one"]))]).toEqual(["two"]);
    expect(toggleSearchResultSelection([accounts[1]], new Set(["one", "two"])).size).toBe(0);
    expect(toggleSearchResultSelection([], new Set(["one"])).size).toBe(0);
  });
});
