// ─── AI 模块共享类型定义 ───
// 所有 AI 相关组件统一从此文件导入接口，避免重复定义

export interface ModelEntry {
  id: string;
  name: string;
  /** 用户自定义启动参数模板（与模型绑定），运行时按需渲染让用户选值 */
  customParams?: ModelCustomParam[];
}

/**
 * 模型自定义启动参数（用户定义）。
 * target='env' 以 envKey 作环境变量注入；target='config' 以 configPath 写入工具配置文件。
 */
export interface ModelCustomParam {
  /** 唯一键（与启动时的取值 key 对应） */
  key: string;
  /** UI 显示名 */
  label: string;
  /** 控件类型：enum | text | bool */
  paramType?: string;
  /** enum 可选值 */
  options?: string[];
  /** 默认值 */
  defaultValue?: string;
  /** 传递目标：env | config */
  target?: string;
  /** env 目标的环境变量名 */
  envKey?: string;
  /** config 目标的 JSON 路径 */
  configPath?: string;
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
  /** 协同模式昵称覆盖 */
  nickname: string | null;
  installed: boolean;
  version: string | null;
  latest_version_cmd?: string;
  latest_version?: string | null;
  install_cmd: string;
  upgrade_cmd: string;
  uninstall_cmd?: string;
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
  /** 进行中操作（"upgrading" | "installing" | "uninstalling"），由后端跟踪，用于持续显示“升级中/安装中” */
  busy?: string | null;
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

// ─── 协同线程（群聊式多工具合作）───

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
  /** 派发耗时（毫秒） */
  duration_ms: number | null;
  /** token 消耗；工具输出含 usage 时回填，否则为 null */
  usage: { input_tokens: number; output_tokens: number } | null;
}

export interface CollabMessage {
  id: string;
  room_id: string;
  /** "user" 或工具 id */
  sender: string;
  sender_name: string;
  /** 展示头像（emoji/单字符），来自工具 config.avatar，旧消息可能为 null */
  avatar: string | null;
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

/** agent 在线状态（每个工具的当前运行状态） */
export interface CollabAgentStatus {
  tool_id: string;
  /** offline | online | thinking | working */
  status: string;
  current_room: string | null;
  last_heartbeat: string;
}

/** 任务流（E）：open/claimed/in_progress/in_review/done */
export interface CollabTask {
  id: string;
  room_id: string;
  title: string;
  description: string;
  status: string;
  assignee: string | null;
  created_by: string;
  parent_task: string | null;
  created_at: string;
  updated_at: string;
}

export interface CollabRoomPage {
  rooms: CollabRoom[];
  has_more: boolean;
  total: number;
}

export interface CollabMessagePage {
  messages: CollabMessage[];
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

/** 协同派发高级协议参数（与工具启动页 LaunchAiToolRequest 对齐） */
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
  /** 模型自定义启动参数模板 */
  custom_params?: ModelCustomParam[];
  /** 用户为模型自定义参数选中的取值（key → 值） */
  custom_param_values?: Record<string, string>;
}

/** 上下文快照：压缩旧会话后生成的摘要 */
export interface ContextSnapshot {
  id: string;
  room_id: string;
  tool_id: string;
  summary: string;
  old_session_id: string;
  message_count: number;
  created_at: string;
}

/** 后端推送：压缩开始（占位消息） */
export interface CollabCompactStartedPayload {
  room_id: string;
  message: CollabMessage;
}

/** 后端推送：压缩完成 */
export interface CollabCompactedPayload {
  room_id: string;
  tool_id: string;
  snapshot: ContextSnapshot | null;
}

/** 代理层推送：请求到达 */
export interface ProxyRequestPayload {
  room_id: string;
  msg_id: string;
  model: string;
  messages: number;
  stream: boolean;
}

/** 代理层推送：上游响应开始 */
export interface ProxyResponseStartPayload {
  room_id: string;
  msg_id: string;
  status: number;
  elapsed_ms: number;
}

/** 代理层推送：流式文本增量 */
export interface ProxyDeltaPayload {
  room_id: string;
  msg_id: string;
  delta: string;
}

/** 代理层推送：响应完成 */
export interface ProxyCompletePayload {
  room_id: string;
  msg_id: string;
  text: string;
  elapsed_ms: number;
}

/** 代理层推送：错误 */
export interface ProxyErrorPayload {
  room_id: string;
  msg_id: string;
  status: number;
  error: string;
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
  /** 本次启动的自定义参数取值 */
  custom_param_values?: Record<string, string>;
  last_launched_at: string;
}
