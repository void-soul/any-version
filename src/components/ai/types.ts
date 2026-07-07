// ─── AI 模块共享类型定义 ───
// 所有 AI 相关组件统一从此文件导入接口，避免重复定义

export interface ModelEntry {
  id: string;
  name: string;
}

export interface ProtocolConfig {
  enabled: boolean;
  url: string;
  use_proxy: boolean;
  model_aliases: Record<string, string>;
  default_model: string | null;
}

export interface AiProvider {
  id: string;
  name: string;
  category: string;
  api_key: string;
  website: string;
  protocols: Record<string, ProtocolConfig>;
  models: ModelEntry[];
  active_model_id: string | null;
}

export interface AiConfig {
  providers: AiProvider[];
  active_provider: string | null;
  proxy_port: number;
  default_project_path: string;
  rectifier: {
    enabled: boolean;
    thinking_signature: boolean;
    thinking_budget: boolean;
    media_fallback: boolean;
  };
  optimizer: {
    enabled: boolean;
    cache_injection: boolean;
    thinking_optimizer: boolean;
    deepseek_normalize: boolean;
  };
  skills_dir: string;
}

export interface DetectedAiTool {
  id: string;
  display_name: string;
  installed: boolean;
  version: string | null;
  latest_version_cmd?: string;
  latest_version?: string | null;
  install_cmd: string;
  upgrade_cmd: string;
  website: string;
  api_protocol: string;
  supports_model: boolean;
  support_one_m_context: boolean;
  supports_fallback_model: boolean;
  resume_cmd: string | null;
  continue_cmd: string | null;
  cache_dirs: string[];
  category: string;
}

export interface AiToolCacheInfo {
  tool_id: string;
  dir_name: string;
  full_path: string;
  size: string;
  size_bytes: number;
  is_junction: boolean;
  junction_target: string;
  exists: boolean;
}

export interface ToolSession {
  session_id: string;
  project_path: string;
  last_used: string;
  summary: string | null;
}

export interface TerminalInfo {
  id: string;
  name: string;
  exe_path: string;
}

export interface LastLaunchConfig {
  provider_id: string | null;
  provider_name: string | null;
  model_id: string | null;
  fallback_model_id: string | null;
  fallback_provider_id: string | null;
  use_official_model: boolean;
  terminal_id: string;
  one_m_context: boolean;
  project_path: string;
  last_launched_at: string;
}
