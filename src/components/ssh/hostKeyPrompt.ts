import { invoke } from "@tauri-apps/api/core";

export interface HostKeyPrompt {
  host: string;
  port: number;
  fingerprint: string;
  decision: "unknown" | "changed";
}

/** 后端用 `__HOST_KEY_PROMPT__:` 前缀把结构化提示塞在错误串里，这里解析出来。 */
export function parseHostKeyPrompt(err: string): HostKeyPrompt | null {
  const prefix = "__HOST_KEY_PROMPT__:";
  if (!err.startsWith(prefix)) return null;
  try {
    const parsed = JSON.parse(err.slice(prefix.length)) as HostKeyPrompt;
    if (!parsed.host || !parsed.fingerprint || !parsed.decision) return null;
    return parsed;
  } catch {
    return null;
  }
}

/**
 * 连接失败时若是因为主机密钥未信任/已变更，展示指纹让用户确认。
 * 返回 true = 用户已确认并写入信任库，调用方应重试连接。
 */
export async function confirmHostKey(
  err: string,
  confirm: (message: string) => boolean,
): Promise<boolean> {
  const prompt = await invoke<HostKeyPrompt | null>("ssh_parse_host_key_prompt", { err });
  if (!prompt) return false;
  const headline = prompt.decision === "changed"
    ? `主机 ${prompt.host}:${prompt.port} 的密钥已变更！这可能是中间人攻击。\n\n新指纹：${prompt.fingerprint}\n\n仅在确认服务器重装/换密钥后继续。`
    : `首次连接 ${prompt.host}:${prompt.port}\n\n主机指纹：${prompt.fingerprint}\n\n请核对服务器指纹后再继续。`;
  if (!confirm(headline)) return false;
  await invoke("ssh_trust_host", {
    host: prompt.host,
    port: prompt.port,
    fingerprint: prompt.fingerprint,
    decision: prompt.decision,
  });
  return true;
}
