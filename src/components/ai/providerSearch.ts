import type { AiProvider } from "./types";

/**
 * 参与匹配的字段：显示名、id、官网、三个协议端点。
 *
 * 与预设选择弹窗的搜索口径一致（那边是 name/id/website/openai_url/anthropic_url），
 * 目的是「按记得的任何线索都能找到人」——供应商多起来之后，用户往往记的是
 * 域名或 id，而不是当初起的中文名。
 */
function matchedFields(provider: AiProvider): string[] {
  return [
    provider.name,
    provider.id,
    provider.website,
    provider.openai_url,
    provider.anthropic_url,
    provider.google_url,
  ];
}

/**
 * 按关键词过滤「已添加的供应商」。
 *
 * 小写子串匹配（大小写不敏感、支持部分匹配）；关键词为空或全空白时**原样返回入参**，
 * 便于调用方省掉一次数组复制。只做展示层过滤，不改动供应商数据本身。
 */
export function filterProviders(providers: AiProvider[], keyword: string): AiProvider[] {
  const kw = keyword.trim().toLowerCase();
  if (!kw) {
    return providers;
  }
  return providers.filter((provider) =>
    matchedFields(provider).some((field) => field.toLowerCase().includes(kw)),
  );
}
