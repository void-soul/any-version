// 对 @tauri-apps/api/core 的包装：重导出原模块全部成员，仅覆盖 invoke，
// 在英文界面下把后端（Rust）抛出的中文错误消息翻译为英文。
//
// 为什么不用 window.__TAURI_INTERNALS__.invoke 包装？
// Tauri v2 在 core.js 里用 Object.defineProperty 定义 invoke（默认 writable=false，
// configurable=false），直接赋值会在生产环境抛
// "Cannot assign to read only property 'invoke'"（见 backendErrors.ts 旧实现）。
// 因此在模块层（Vite alias 指向本文件）拦截，所有 `import { invoke } from "@tauri-apps/api/core"`
// 都会走到这里的包装版本。
//
// 注意：这里通过相对路径引用真正的 core.js，而不是 "@tauri-apps/api/core" ——
// 否则会被本模块自己的 Vite alias 再次命而形成无限递归。
import type { InvokeArgs, InvokeOptions } from "../node_modules/@tauri-apps/api/core.js";
import { invoke as rawInvoke } from "../node_modules/@tauri-apps/api/core.js";
import { translateErrorIfEn } from "./i18n/backendErrors";

export {
  Channel,
  PluginListener,
  Resource,
  SERIALIZE_TO_IPC_FN,
  addPluginListener,
  checkPermissions,
  convertFileSrc,
  isTauri,
  requestPermissions,
  transformCallback,
} from "../node_modules/@tauri-apps/api/core.js";

/**
 * 类型化 invoke：reject 时若为英文界面，把后端中文错误消息翻译为英文，
 * 原始中文保留在 error.rawMessage 供精确判断。
 */
export async function invoke<T = unknown>(
  cmd: string,
  args?: InvokeArgs,
  options?: InvokeOptions,
): Promise<T> {
  try {
    return await rawInvoke<T>(cmd, args, options);
  } catch (e: unknown) {
    const msg = typeof e === "string" ? e : (e as Error)?.message ?? String(e);
    const en = translateErrorIfEn(msg);
    if (en !== msg && en) {
      if (typeof e === "string") {
        const err = new Error(en);
        (err as Error & { rawMessage?: string }).rawMessage = msg;
        throw err;
      }
      if (e instanceof Error) {
        (e as Error & { rawMessage?: string }).rawMessage = e.message;
        e.message = en;
      }
    }
    throw e;
  }
}