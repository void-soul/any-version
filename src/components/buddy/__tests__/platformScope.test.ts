import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import type { BuddySwitchProgress } from "../BuddyPanel.model";
import { createBuddyPlatformScope } from "../platformScope";
import { loadBuddyPlatform } from "../loadBuddyPlatform";
import { runBuddyAccountSwitch } from "../runBuddyAccountSwitch";
import { useBuddyAccountMaintenance } from "../useBuddyAccountMaintenance";
import { useBuddyAccountImport } from "../useBuddyAccountImport";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(), stateIndex: 0, values: new Map<number, unknown>(), setters: [] as ReturnType<typeof vi.fn>[] }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
// Exercise the actual workflow closures with controlled hook state, without a DOM or native bridge.
vi.mock("react", async importOriginal => ({ ...await importOriginal<typeof import("react")>(),
  useLayoutEffect: (effect: () => void) => { effect(); },
  useMemo: (factory: () => unknown) => factory(),
  useState: (initial: unknown) => {
    const index = mocks.stateIndex++;
    const setter = vi.fn(); mocks.setters.push(setter);
    return [mocks.values.has(index) ? mocks.values.get(index) : typeof initial === "function" ? initial() : initial, setter];
  },
}));
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (error: unknown) => void; const promise = new Promise<T>((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; }
function scope() { const value = createBuddyPlatformScope(); value.activate(); return value; }
function button(node: ReactNode, label: string): (() => void) | undefined {
  if (Array.isArray(node)) { for (const child of [...node].reverse()) { const found = button(child, label); if (found) return found; } return; }
  if (!isValidElement<{ children?: ReactNode; onClick?: () => void }>(node)) return;
  const text = (children: ReactNode): string => Array.isArray(children) ? children.map(text).join("") : typeof children === "string" ? children : "";
  if (node.type === "button" && text(node.props.children).includes(label)) return node.props.onClick;
  return button(node.props.children, label);
}
beforeEach(() => { vi.clearAllMocks(); mocks.invoke.mockReset(); mocks.listen.mockReset(); mocks.stateIndex = 0; mocks.values.clear(); mocks.setters.length = 0; });

describe("Buddy platform operation isolation", () => {
  it("does not reauthorize the old A after A → B → A or effect replay", () => {
    const a = scope(), old = a.capture(); a.dispose(); const b = scope(); b.dispose(); const newA = scope();
    expect(old.current()).toBe(false); expect(newA.capture().current()).toBe(true);
    const firstMount = newA.capture(); newA.dispose(); newA.activate(); expect(firstMount.current()).toBe(false);
  });
  it("rejects an old loader before it can read a new epoch or issue queries", async () => {
    const a = scope(); const load = () => loadBuddyPlatform({ scope: a, platform: "workbuddy", onLoaded: vi.fn(), onError: vi.fn(), onFinished: vi.fn() });
    a.dispose(); scope(); await load(); expect(mocks.invoke).not.toHaveBeenCalled();
  });
  it.each([false, true])("discards a delayed list result/error after switch (error=%s)", async fail => {
    const pending = deferred<unknown>(), a = scope(), onLoaded = vi.fn(), onError = vi.fn(), onFinished = vi.fn();
    mocks.invoke.mockReturnValue(pending.promise);
    const run = loadBuddyPlatform({ scope: a, platform: "workbuddy", onLoaded, onError, onFinished });
    a.dispose(); if (fail) pending.reject(new Error("old platform failure")); else pending.resolve([]);
    await run; expect(onLoaded).not.toHaveBeenCalled(); expect(onError).not.toHaveBeenCalled(); expect(onFinished).not.toHaveBeenCalled();
  });
  it.each([false, true])("maintenance completion cannot reload, report or clear newer busy (error=%s)", async fail => {
    const pending = deferred<unknown>(), a = scope(), setBusy = vi.fn(), showMsg = vi.fn(), load = vi.fn(); mocks.invoke.mockReturnValue(pending.promise);
    const actions = useBuddyAccountMaintenance({ platformScope: a, platform: "workbuddy", accounts: [], currentId: null, setBusy, setMessage: vi.fn(), showMsg, load });
    const run = actions.refreshAccount("synthetic-id"); expect(setBusy).toHaveBeenCalledWith(true); setBusy.mockClear();
    a.dispose(); if (fail) pending.reject(new Error("late")); else pending.resolve({}); await run;
    expect(load).not.toHaveBeenCalled(); expect(showMsg).not.toHaveBeenCalled(); expect(setBusy).not.toHaveBeenCalled();
  });
  it.each(["oauth", "token", "local"])("ignores delayed %s import state and follow-up load after unmount", async mode => {
    const pending = deferred<unknown>(), a = scope(), setBusy = vi.fn(), showMsg = vi.fn(), load = vi.fn(); mocks.invoke.mockReturnValue(pending.promise);
    mocks.values.set(0, true); mocks.values.set(1, mode); mocks.values.set(2, { loginId: "synthetic-login" }); mocks.values.set(4, "synthetic-token");
    const actions = useBuddyAccountImport({ platformScope: a, platform: "workbuddy", platformLabel: "WorkBuddy", busy: false, setBusy, setMessage: vi.fn(), showMsg, load, displayName: () => "synthetic" });
    const click = button(actions.renderAddAccountDialog(), mode === "oauth" ? "buddy.oauthComplete" : mode === "token" ? "buddy.addTokenSubmit" : "buddy.importLocal"); expect(click).toBeDefined(); click!();
    await vi.waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(1)); const counts = mocks.setters.map(fn => fn.mock.calls.length); setBusy.mockClear(); a.dispose(); pending.resolve({});
    await new Promise(resolve => setTimeout(resolve, 0)); expect(load).not.toHaveBeenCalled(); expect(showMsg).not.toHaveBeenCalled(); expect(setBusy).not.toHaveBeenCalled(); expect(mocks.setters.map(fn => fn.mock.calls.length)).toEqual(counts);
  });
  it("still applies and finishes the active platform snapshot", async () => {
    mocks.invoke.mockImplementation((command: string) => Promise.resolve(command === "buddy_list_accounts" ? [] : command === "buddy_get_current_account_id" ? "active" : {}));
    const onLoaded = vi.fn(), onFinished = vi.fn(), onError = vi.fn();
    await loadBuddyPlatform({ scope: scope(), platform: "workbuddy", onLoaded, onFinished, onError });
    expect(onLoaded).toHaveBeenCalledWith(expect.objectContaining({ currentId: "active" })); expect(onFinished).toHaveBeenCalledOnce(); expect(onError).not.toHaveBeenCalled();
  });
  it("still reloads after a successful active maintenance operation", async () => {
    mocks.invoke.mockResolvedValue({}); const setBusy = vi.fn(), showMsg = vi.fn(), load = vi.fn().mockResolvedValue(undefined);
    const actions = useBuddyAccountMaintenance({ platformScope: scope(), platform: "workbuddy", accounts: [], currentId: null, setBusy, setMessage: vi.fn(), showMsg, load });
    await actions.refreshAccount("fake"); expect(load).toHaveBeenCalledOnce(); expect(showMsg).toHaveBeenCalledWith(true, "buddy.refreshed"); expect(setBusy).toHaveBeenLastCalledWith(false);
  });
  it("resets OAuth busy/login state for a new platform lifetime", () => {
    mocks.values.set(2, { loginId: "old-platform" }); mocks.values.set(3, true);
    useBuddyAccountImport({ platformScope: scope(), platform: "codebuddy-cn", platformLabel: "CN", busy: false, setBusy: vi.fn(), setMessage: vi.fn(), showMsg: vi.fn(), load: vi.fn(), displayName: () => "fake" });
    expect(mocks.setters[2]).toHaveBeenCalledWith(null); expect(mocks.setters[3]).toHaveBeenCalledWith(false);
  });
  it("only consumes progress for the current switch and stops consuming on disposal", async () => {
    const pending = deferred<[string, null]>(), a = scope(), onProgress = vi.fn(); let event!: (event: { payload: BuddySwitchProgress }) => void;
    mocks.listen.mockImplementation(async (_name, handler) => { event = handler; return vi.fn(); }); mocks.invoke.mockReturnValue(pending.promise);
    const run = runBuddyAccountSwitch({ scope: a, platform: "workbuddy", accountId: "fake", onProgress, onResult: vi.fn(), onError: vi.fn(), onFinished: vi.fn() });
    await vi.waitFor(() => expect(mocks.invoke).toHaveBeenCalled());
    const payload: BuddySwitchProgress = { platform: "workbuddy", accountId: "fake", stage: "checking", scannedWorkspaces: 0 };
    event({ payload: { ...payload, platform: "codebuddy-cn" } }); event({ payload: { ...payload, accountId: "other" } }); expect(onProgress).not.toHaveBeenCalled();
    event({ payload }); expect(onProgress).toHaveBeenCalledOnce(); a.dispose(); event({ payload }); expect(onProgress).toHaveBeenCalledOnce(); pending.resolve(["late", null]); await run;
  });
  it("cleans up late listener registration without starting a stale switch", async () => {
    const pending = deferred<() => void>(), a = scope(), unlisten = vi.fn(); mocks.listen.mockReturnValue(pending.promise);
    const onResult = vi.fn(), onFinished = vi.fn(); const run = runBuddyAccountSwitch({ scope: a, platform: "workbuddy", accountId: "fake", onProgress: vi.fn(), onResult, onError: vi.fn(), onFinished });
    a.dispose(); pending.resolve(unlisten); await run; expect(unlisten).toHaveBeenCalledTimes(1); expect(mocks.invoke).not.toHaveBeenCalled(); expect(onFinished).not.toHaveBeenCalled(); expect(onResult).not.toHaveBeenCalled();
  });
  it("detaches an active switch listener immediately on unmount and ignores its late result", async () => {
    const pending = deferred<[string, null]>(), a = scope(), unlisten = vi.fn(), onProgress = vi.fn(), onResult = vi.fn(), onFinished = vi.fn(); mocks.listen.mockResolvedValue(unlisten); mocks.invoke.mockReturnValue(pending.promise);
    const run = runBuddyAccountSwitch({ scope: a, platform: "workbuddy", accountId: "fake", onProgress, onResult, onError: vi.fn(), onFinished });
    await vi.waitFor(() => expect(mocks.invoke).toHaveBeenCalled()); a.dispose(); expect(unlisten).toHaveBeenCalledTimes(1); pending.resolve(["late", null]); await run;
    expect(onResult).not.toHaveBeenCalled(); expect(onFinished).not.toHaveBeenCalled(); expect(unlisten).toHaveBeenCalledTimes(1);
  });
});
