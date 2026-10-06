// ─── 模型自定义参数预设库 ───
// 「从预设添加」下拉的来源：把高频、且确实能在工具里生效的参数做成模板，
// 用户选一下即可，不用手敲 key / label / paramType / options / configPath。
//
// 目前只收录 Codex 系（Codex CLI / ChatGPT Desktop）的 `~/.codex/config.toml` 顶层键：
// - 这几个键在 `tool_config_extras/codex.rs` 里原本是**写死**的（思考强度恒 high、
//   上下文窗口按模型查表），用户自定义的值会被覆盖 —— 那正是「填了不生效」的摆设；
//   配套后端改动（`ExtrasCtx::user_configured_paths`）已让写死逻辑对用户自定义让位。
// - 别的工具的配置键没有可靠来源（原生配置 schema 各异、版本还在变），
//   宁可少给，也不能给一条「选了不生效」的模板。
//
// `param.label` 是**用户数据**（保存进配置、可在模型卡片里继续编辑），所以这里给中文
// 初始值而不是 i18n key；下拉里的 `name` / `scope` 才走 i18n。

import type { ModelCustomParam } from "./types";

export interface ParamPreset {
  id: string;
  /** 下拉里显示的名字（i18n key） */
  nameKey: string;
  /** 适用工具说明（i18n key） */
  scopeKey: string;
  /** 选它生成的参数模板 */
  param: ModelCustomParam;
}

export const PARAM_PRESETS: ParamPreset[] = [
  {
    id: "codex-reasoning-effort",
    nameKey: "modelcfg.presetReasoningEffort",
    scopeKey: "modelcfg.presetScopeCodex",
    param: {
      key: "reasoning_effort",
      label: "思考强度",
      paramType: "enum",
      options: ["minimal", "low", "medium", "high"],
      defaultValue: "high",
      target: "config",
      configPath: "model_reasoning_effort",
    },
  },
  {
    id: "codex-context-window",
    nameKey: "modelcfg.presetContextWindow",
    scopeKey: "modelcfg.presetScopeCodex",
    param: {
      key: "context_window",
      label: "上下文窗口",
      paramType: "text",
      target: "config",
      configPath: "model_context_window",
    },
  },
  {
    id: "codex-auto-compact",
    nameKey: "modelcfg.presetAutoCompact",
    scopeKey: "modelcfg.presetScopeCodex",
    param: {
      key: "auto_compact_token_limit",
      label: "自动压缩 token 上限",
      paramType: "text",
      target: "config",
      configPath: "model_auto_compact_token_limit",
    },
  },
];
