import type { AiProvider } from "../../contracts/ai";
export type Preset = {
  id: string; name: string; category: string;
  website: string; openai_url: string; anthropic_url: string;
  google_url: string;
};


export const EMPTY_PROVIDER: AiProvider = {
  id: "", name: "", category: "provider", api_key: "", website: "",
  openai_url: "", anthropic_url: "", google_url: "",
  models: [], active_model_id: null, custom_headers: [],
  openai_include_v1: null, anthropic_include_v1: null,
  promotions: [],
};


/// 「是否包含 /v1」三态 → 下拉框字符串（null 即自动）。
export function v1ToSelect(v?: boolean | null): string {
  return v === true ? "yes" : v === false ? "no" : "auto";
}

export function selectToV1(s: string): boolean | null {
  return s === "yes" ? true : s === "no" ? false : null;
}


/// 传输层 / 逐跳头：由 HTTP 客户端按实际报文决定，后端也会拒绝，这里提前拦。
export const FORBIDDEN_HEADER_NAMES = new Set([
  "host", "content-length", "transfer-encoding", "connection", "keep-alive",
  "proxy-connection", "te", "trailer", "upgrade", "expect",
]);


/// RFC 7230 token 字符集（HTTP 头名称的合法字符）
export const HEADER_NAME_RE = /^[A-Za-z0-9!#$%&'*+\-.^_`|~]+$/;


/// 从预设（可能含多个协议端点）取出全部协议 URL
export function presetUrls(p: Preset): { openai_url: string; anthropic_url: string; google_url: string } {
  return { openai_url: p.openai_url, anthropic_url: p.anthropic_url, google_url: p.google_url };
}
