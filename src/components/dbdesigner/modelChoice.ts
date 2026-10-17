// 「选模型 → 记住 → 下次沿用」这条链的纯逻辑。
//
// 为什么单独成模块：这个模块面板带 i18n / Tauri / React Flow 依赖，
// 单测根本拖不进来；而这条链恰恰是**唯一一处静默退化过的地方**：
// Q-0421 把 AI 面板重写成「窄条 + 气泡」时，输入框旁的模型选择器连同
// onSelect 回调一起没了 —— 那段回调是唯一往 `dbd_save_settings` 写
// lastProviderId/lastModelId 的代码。结果一次面板重写就同时带走了
// 「用户能选模型」和「上次选的被记住」两件事，而 tsc 不报错、
// 纯逻辑测试也测不到（Q-0424 补输入框时就是这么漏的）。
//
// 所以把规则落成纯函数：规则一旦有测试，再断就会被抓住。
//
// 沿用规则只有一条，但它值得单测：**记住的选择必须先验证还存不存在**。
// 供应商或模型被删掉后直接套用会得到一个空 id，后端只能回退默认，
// 而用户界面上一无所知 —— 他以为自己还选着它。

/** 只取纯逻辑真正用到的字段：拖整个 AiProviderLike 进来会让测试依赖后端类型 */
export interface ChoiceProvider {
  id: string;
  active_model_id?: string | null;
  models?: { id: string; name?: string }[];
}

/** `dbd_get_settings` 里记着的上次选择（可能一个字段都没有） */
export interface RememberedModel {
  lastProviderId?: string;
  lastModelId?: string;
}

export interface ModelChoiceInput {
  providers: ChoiceProvider[];
  remembered?: RememberedModel | null;
}

export interface ModelChoice {
  providerId: string;
  modelId: string;
  /**
   * 是不是真的沿用了记住的选择（而不是回落默认）。
   *
   * 调用方据此决定要不要回写设置：**只有真的沿用了才需要写回去** ——
   * 已经写着的值不必再写，而回落后的值若原样保留，会让下次启动继续
   * 拿着一个已失效的 id 去「沿用」，问题原地复发。
   */
  usedRemembered: boolean;
}

/**
 * 定出这次该用哪个供应商 + 哪个模型。**没有可用供应商时返回 null**。
 *
 * 回落顺序（逐级、每级都先验证存在）：
 *   1. 记住的供应商 + 记住的模型（都还在）→ 原样沿用
 *   2. 记住的供应商（还在）+ 该供应商的 active model
 *   3. 第一个供应商 + 它的 active model
 *   4. 第一个供应商 + models[0]
 *   5. 仍拿不到模型 → modelId 为空串（**不是 null**）
 */
export function resolveModelChoice(input: ModelChoiceInput): ModelChoice | null {
  const { providers, remembered } = input;
  if (providers.length === 0) return null;

  const rememberedProv = remembered?.lastProviderId
    ? providers.find((p) => p.id === remembered.lastProviderId)
    : undefined;
  // 第 1 级：两个 id 都还成立，才算真的沿用
  if (
    rememberedProv &&
    remembered?.lastModelId &&
    rememberedProv.models?.some((m) => m.id === remembered.lastModelId)
  ) {
    return { providerId: rememberedProv.id, modelId: remembered.lastModelId, usedRemembered: true };
  }

  const prov = rememberedProv ?? providers[0];
  // 第 5 级：providerId 已定但模型未定 —— 返回空串而不是 null。
  // 返回 null 会让界面误判成「没配供应商」而禁用提交，可供应商明明是配好的。
  const active = prov.active_model_id ?? null;
  const modelId =
    active && prov.models?.some((m) => m.id === active)
      ? active
      : prov.models?.[0]?.id ?? "";
  return { providerId: prov.id, modelId, usedRemembered: false };
}

/**
 * 切换模型后要写回 `dbd_save_settings` 的载荷。
 *
 * **不持久化不行**：主模块切换是 `<Activity>` keep-alive，切回时挂载 effect
 * 会重跑一遍「读设置 + 决定用哪个模型」，不写回的话那次重跑会把你刚选好的
 * 模型覆盖回默认。
 *
 * 空 `modelId` 也要照写（见测试里的理由）。
 */
export function modelChoiceSettings(choice: {
  providerId: string;
  modelId: string;
}): Required<RememberedModel> {
  return { lastProviderId: choice.providerId, lastModelId: choice.modelId };
}