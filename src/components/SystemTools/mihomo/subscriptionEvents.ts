import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 后端 lifecycle.rs `auto_update_profiles` 在订阅更新完成后 emit 的事件。 */
export const SUBSCRIPTION_UPDATED_EVENT = "mihomo://subscription-updated";

/**
 * 注册监听并返回同步 cleanup，可直接作为 useEffect 的返回值。
 * 卸载可能发生在 await listen(...) 返回之前，此时需就地注销，避免监听器泄漏。
 */
export function listenSubscriptionUpdated(onUpdated: () => void): () => void {
  let cancelled = false;
  let un: UnlistenFn | undefined;
  void listen(SUBSCRIPTION_UPDATED_EVENT, onUpdated).then((fn) => {
    if (cancelled) fn();
    else un = fn;
  });
  return () => { cancelled = true; un?.(); };
}
