/**
 * 统一的 IPC 失败可观测封装（AUDIT-016）。
 *
 * 项目里曾散布 84 处 `.catch(() => {})` / `catch {}`，失败既不提示用户也不写日志，
 * 而 UI 往往已乐观更新，表现为「点了没反应 / 配置存不进去」。所有写路径必须
 * 走这里：至少留下 console.warn（供日志查看器采集），可选 Toast。
 *
 * 只有**确无反馈价值**的路径（卸载清理、浏览器预览无 Tauri 后端、剪贴板权限探测）
 * 才允许裸忽略，且必须用 `ignoreInvokeError` 显式标注理由，供静态闸识别。
 */
import { toast } from "../components/shared/Toast";

/** 已上报错误的去重：同一命令短时间内连续失败不刷屏（仍会 console.warn）。 */
const recent = new Map<string, number>();
const DEDUPE_MS = 1500;

export interface ReportOptions {
  /** 命令名或操作描述，用于日志与提示前缀。 */
  context: string;
  /** 是否弹用户可见提示（默认 false，仅记日志）。写操作建议 true。 */
  notify?: boolean;
  /** 用户可见文案；不传时显示通用提示。 */
  message?: string;
}

/**
 * 记录一次 IPC 失败。永远不抛异常，不吞掉原始信息。
 * 返回归一化后的错误字符串，便于调用方按需处理。
 */
export function reportInvokeError(error: unknown, options: ReportOptions | string): string {
  const opts: ReportOptions = typeof options === "string" ? { context: options } : options;
  const raw = error instanceof Error ? error.message : String(error);
  console.warn(`[invoke] ${opts.context} 失败:`, error);

  if (opts.notify) {
    const key = `${opts.context}:${raw}`;
    const now = Date.now();
    const last = recent.get(key) ?? 0;
    if (now - last > DEDUPE_MS) {
      recent.set(key, now);
      toast(opts.message ?? `${opts.context} 失败，请重试`, "err");
    }
    if (recent.size > 200) {
      for (const [k, t] of recent) if (now - t > DEDUPE_MS * 10) recent.delete(k);
    }
  }
  return raw;
}

/**
 * 显式标注「此处失败确实无需上报」。仅供卸载清理 / 浏览器预览降级 /
 * 能力探测使用；静态闸会要求写明理由参数。
 */
export function ignoreInvokeError(_error: unknown, _reason: string): void {
  // 有意为空：调用点已声明为什么可以安全忽略。
}

/** 包一层：失败自动上报，成功原样返回。 */
export async function invokeOrReport<T>(
  promise: Promise<T>,
  options: ReportOptions | string,
): Promise<T | undefined> {
  try {
    return await promise;
  } catch (error) {
    reportInvokeError(error, options);
    return undefined;
  }
}
