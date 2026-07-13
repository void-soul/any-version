// ─── AI 模块共享类型定义 ───
// 所有 AI 相关组件统一从此文件导入接口，避免重复定义

export interface ModelEntry {
  id: string;
  name: string;
}

export interface AiProvider {
  id: string;
  name: string;
  category: string;
  api_key: string;
  website: string;
  /** OpenAI 协议端点 URL（空字符串表示供应商不支持该协议） */
  openai_url: string;
  /** Anthropic 协议端点 URL（空字符串表示供应商不支持该协议） */
  anthropic_url: string;
  /** Google 协议端点 URL（空字符串表示供应商不支持该协议） */
  google_url: string;
  models: ModelEntry[];
  active_model_id: string | null;
}

export interface ProviderPreset {
  id: string;
  name: string;
  category: string;
  website: string;
  /** 预设支持的所有协议端点（catalog 用，实例化时择一） */
  openai_url: string;
  anthropic_url: string;
  google_url: string;
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
    protocol_mismatch: boolean;
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
  /** 协作模式头像（emoji 或单字符） */
  avatar: string | null;
  /** 协作模式昵称覆盖 */
  nickname: string | null;
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
  supports_openai: boolean;
  supports_anthropic: boolean;
  supports_google: boolean;
  builtin_models: string[];
  supports_optimizer: boolean;
  supports_rectifier: boolean;
  /** MSIX/Store 启动 URI（无普通 exe 时使用） */
  launch_uri: string | null;
  /** 检测到的可执行文件路径（GUI/桌面应用启动用） */
  detected_path: string | null;
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

// ─── 协作线程（群聊式多工具合作）───

export interface CollabReference {
  source_message_id: string;
  source_sender_name: string;
  excerpt: string;
}

export interface CollabFileRef {
  path: string;
}

export interface CollabDispatch {
  tool_id: string;
  session_id: string;
  model: string | null;
}

export interface CollabMessage {
  id: string;
  room_id: string;
  /** "user" 或工具 id */
  sender: string;
  sender_name: string;
  content: string;
  references: CollabReference[];
  files: CollabFileRef[];
  dispatch: CollabDispatch | null;
  reply_to: string | null;
  /** 工具消息状态："running" | "done" | "error" */
  status: string | null;
  created_at: string;
}

export interface CollabRoom {
  id: string;
  name: string;
  project_path: string;
  created_at: string;
  updated_at: string;
}

export interface CollabRoomPage {
  rooms: CollabRoom[];
  has_more: boolean;
  total: number;
}

/** 后端流式推送：增量文本 */
export interface CollabDeltaPayload {
  room_id: string;
  msg_id: string;
  delta: string;
}

/** 后端流式推送：活动状态（思考中/调用工具等） */
export interface CollabActivityPayload {
  room_id: string;
  msg_id: string;
  activity: string;
}

/** 后端推送：工具询问用户选择 */
export interface CollabPromptPayload {
  room_id: string;
  msg_id: string;
  question: string;
  options: string[];
}

/** 后端流式推送：某条消息收尾（含 done/error 状态） */
export interface CollabMsgUpdatedPayload {
  room_id: string;
  message: CollabMessage;
}

/** 协作派发高级协议参数（与工具启动页 LaunchAiToolRequest 对齐） */
export interface CollabDispatchOptions {
  masquerade_model: string | null;
  fallback_model_id: string | null;
  fallback_provider_id: string | null;
  fallback_masquerade_model: string | null;
  one_m_context: boolean;
  fallback_one_m_context: boolean;
  optimizer_enabled: boolean | null;
  rectifier_enabled: boolean | null;
  optimizer_cache_injection: boolean | null;
  optimizer_thinking: boolean | null;
  optimizer_deepseek: boolean | null;
  rectifier_thinking_signature: boolean | null;
  rectifier_thinking_budget: boolean | null;
  rectifier_media_fallback: boolean | null;
  rectifier_protocol_mismatch: boolean | null;
}

export interface LastLaunchConfig {
  provider_id: string | null;
  provider_name: string | null;
  model_id: string | null;
  fallback_model_id: string | null;
  fallback_provider_id: string | null;
  /** fallback/小模型的伪装声明名 C，空表示不伪装 */
  fallback_masquerade_model: string | null;
  use_official_model: boolean;
  terminal_id: string;
  one_m_context: boolean;
  /** fallback/小模型是否同样追加 [1m] */
  fallback_one_m_context: boolean;
  project_path: string;
  /** 模型伪装：工具以为自己调用的模型名 C，空表示不伪装 */
  masquerade_model: string | null;
  /** 本次启动是否启用优化器 */
  optimizer_enabled: boolean | null;
  /** 本次启动是否启用整流器 */
  rectifier_enabled: boolean | null;
  last_launched_at: string;
}
