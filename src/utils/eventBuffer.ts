/**
 * 模块级 Tauri 事件缓冲存储（keep-alive 配套设施）。
 *
 * 背景：页面用 <Activity> keep-alive 后，面板隐藏时其内部 Effects 会被销毁，
 * 组件内的 Tauri listen() 订阅随之失效；而后端 emit 是「即发即弃」的，
 * 隐藏/未挂载期间的进度事件（如思维导图 AI 导入的 mm-ai-progress）会永久丢失。
 *
 * 本工具在【模块作用域】（App 生命周期）注册唯一的事件订阅，把载荷快照存进
 * 环形缓冲区；面板组件通过 useSyncExternalStore 订阅缓冲区，重新挂载/切回时
 * 立即拿到完整历史并继续实时接收——切换页面不再丢失任何进度。
 *
 * 用法：
 *   // 模块顶层创建（每个事件全局单例：同名的重复调用返回同一缓冲实例）
 *   const mmAiBuffer = createEventBuffer("mm-ai-progress");
 *   // 组件内
 *   const entries = useEventBufferSnapshot(mmAiBuffer);
 *
 * 性能：环形缓冲区有上限（默认 500 条，超出自动丢弃最旧条目）；notify 用微任务
 * 合并，同一轮事件风暴只触发一次 React 重渲染；缓冲区内仅存载荷浅快照，无 DOM。
 */

import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";

export interface EventBuffer<T = unknown> {
  /** 当前缓冲的条目数（≤ limit） */
  readonly size: number;
  /** 缓冲快照（只读；内容变化时返回新数组，无变化时引用稳定） */
  snapshot(): readonly T[];
  /** 缓冲版本号：每次缓冲内容变化（含 clear）自增，用作 getSnapshot 缓存键 */
  version(): number;
  /** 订阅缓冲变化（返回退订函数）；变化按微任务合并，事件风暴只通知一次 */
  subscribe(fn: () => void): () => void;
  /** 清空缓冲（通知订阅者；用于新一轮任务开始前重置） */
  clear(): void;
}

export interface EventBufferOptions<T> {
  /** 缓冲上限，超出丢弃最旧条目；默认 500 */
  limit?: number;
  /** 载荷 → 存储条目的转换；默认原样存储 */
  transform?: (payload: unknown) => T;
}

// 同一 Tauri 事件 → 全局唯一缓冲实例（单例注册表）。
const buffersByEvent = new Map<string, EventBuffer<unknown>>();

/**
 * 为某个 Tauri 事件创建（或复用）模块级缓冲。
 * 同一事件名重复调用返回同一个实例（后端也只挂一次订阅）；首个调用的 options 生效。
 * 后端订阅是异步建立的（listen 返回 Promise）；建立前的极早期事件同样会丢失，
 * 但本函数在首次 import 时即被调用，远早于任何用户操作，实践中无窗口期。
 */
export function createEventBuffer<T = unknown>(
  event: string,
  opts: EventBufferOptions<T> = {}
): EventBuffer<T> {
  const existing = buffersByEvent.get(event) as EventBuffer<T> | undefined;
  if (existing) return existing;

  const limit = Math.max(1, opts.limit ?? 500);
  const transform = opts.transform ?? ((p: unknown) => p as T);

  let items: readonly T[] = [];
  let version = 0;
  const subscribers = new Set<() => void>();

  // 微任务合并：一轮同步到达的 N 个事件只通知一次订阅者（React 一次重渲染）。
  let notifyQueued = false;
  const scheduleNotify = () => {
    if (notifyQueued) return;
    notifyQueued = true;
    queueMicrotask(() => {
      notifyQueued = false;
      version++;
      subscribers.forEach((fn) => {
        try {
          fn();
        } catch (e) {
          console.error(`[eventBuffer:${event}] 订阅者异常`, e);
        }
      });
    });
  };

  const push = (payload: unknown) => {
    let entry: T;
    try {
      entry = transform(payload);
    } catch (e) {
      console.warn(`[eventBuffer:${event}] transform 失败，忽略该事件`, e);
      return;
    }
    // 不可变更新：旧快照引用保持不变（useSyncExternalStore 依赖引用稳定性）。
    // 上限 500 条，O(n) 拷贝在事件频率（≤5/s 量级）下可忽略。
    const next = items.length >= limit ? items.slice(items.length - limit + 1) : items;
    items = [...next, entry];
    scheduleNotify();
  };

  const buffer: EventBuffer<T> = {
    get size() {
      return items.length;
    },
    snapshot: () => items,
    version: () => version,
    subscribe: (fn) => {
      subscribers.add(fn);
      return () => {
        subscribers.delete(fn);
      };
    },
    clear: () => {
      if (items.length === 0) return;
      items = [];
      scheduleNotify();
    },
  };

  buffersByEvent.set(event, buffer as EventBuffer<unknown>);
  listen(event, (e) => push(e.payload)).catch((err) => {
    buffersByEvent.delete(event);
    console.error(`[eventBuffer:${event}] 注册事件监听失败`, err);
  });
  return buffer;
}

/**
 * 在组件内消费缓冲区：等价 useSyncExternalStore(subscribe, getSnapshot)。
 * getSnapshot 以版本号为键做缓存（WeakMap 按缓冲实例隔离）：内容没变时返回同一
 * 引用（useSyncExternalStore 要求快照引用稳定），内容变了返回新数组。
 */
const snapshotCaches = new WeakMap<
  EventBuffer<unknown>,
  { version: number; snapshot: readonly unknown[] }
>();

export function useEventBufferSnapshot<T>(buffer: EventBuffer<T>): readonly T[] {
  return useSyncExternalStore(
    buffer.subscribe,
    () => {
      let cache = snapshotCaches.get(buffer as EventBuffer<unknown>);
      const v = buffer.version();
      if (!cache || cache.version !== v) {
        cache = { version: v, snapshot: buffer.snapshot() };
        snapshotCaches.set(buffer as EventBuffer<unknown>, cache);
      }
      return cache.snapshot;
    }
  ) as readonly T[];
}
