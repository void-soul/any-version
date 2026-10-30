
// 记住当前项目/环境/激活接口（模块卸载重挂载后恢复）
export const API_CTX_KEY = "any_version_api_ctx";

/** 浮层底：面板色 + 微量透明度渐变（统一走皮肤 token，勿再写死颜色） */
export const POPOVER_BG =
  "linear-gradient(160deg, color-mix(in srgb, var(--color-surface-panel) 99%, transparent), color-mix(in srgb, var(--color-surface-panel) 95%, transparent))";
