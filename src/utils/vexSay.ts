// ─── Kira 事件伴随语（全局小喇叭） ───
// 后台干完大事/出错时，任何模块调用 vexSay() 就能让 vex 开口吐槽/报喜，
// 由 App 顶部统一渲染成 Kira 风格 toast，全 App 一套语气。

export type VexSayKind = "info" | "success" | "error";

export type VexSayListener = (msg: string, kind: VexSayKind) => void;

const listeners = new Set<VexSayListener>();

/** 让 Kira 开口说一句（info / success / error 三种语气）。 */
export function vexSay(msg: string, kind: VexSayKind = "info"): void {
  listeners.forEach((l) => {
    try {
      l(msg, kind);
    } catch {
      /* 单个监听器出错不影响其它 */
    }
  });
}

/** 订阅 Kira 说的话；返回取消订阅函数。 */
export function onVexSay(fn: VexSayListener): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}