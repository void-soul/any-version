/**
 * 清除 ANSI 转义序列与控制字符。
 * 用于日志复制、纯文本导出等场景，避免把颜色/光标控制码带进剪贴板或导出文件。
 */
export function stripAnsi(s: string): string {
  return s
    .replace(/\x1b\[[0-9;]*[A-Za-z]/g, "")
    .replace(/[\x00-\x08\x0a-\x0d\x0e-\x1f\x7f]/g, "");
}
