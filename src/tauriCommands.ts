// 集中封装 Tauri `invoke`，统一类型与错误处理。
// 逐步将散落在各组件中的 invoke("command_name", {...}) 迁移到此处，
// 以避免命令重命名/参数变更时前端调用方无编译期约束。
import { invoke as tauriInvoke } from "@tauri-apps/api/core";

/** 带泛型的类型化 invoke 封装。 */
export async function invoke<T = void>(
  cmd: string,
  args?: Record<string, unknown>
): Promise<T> {
  return tauriInvoke<T>(cmd, args);
}

// ---- 高频命令的显式类型封装（按需补充）----

/** 执行 shell 命令并捕获输出（注意：命令来源须受白名单约束，见后端 T1 修复）。 */
export const runCmdCapture = (cmd: string, projectId?: string) =>
  invoke<string>("run_cmd_capture", { cmd, projectId: projectId ?? null });

/** 清空 mihomo 运行时告警。 */
export const mihomoClearWarnings = () => invoke("mihomo_clear_warnings");
