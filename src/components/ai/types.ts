// ─── AI 模块共享类型定义 ───
// 所有 AI 相关组件统一从此文件导入接口，避免重复定义

export interface ModelEntry {
  id: string;
  name: string;
  /** 用户自定义启动参数模板（与模型绑定），运行时按需渲染让用户选值 */
  customParams?: ModelCustomParam[];
}

/** 供应商的促销活动倒计时（用户手动登记）：到期置灰、保留 7 天后由前端惰性清除。 */
export interface ProviderPromotion {
  id: string;
  /** 活动名称，如「Gemini 2.5 Flash 免费 1 个月」 */
  name: string;
  /** 到期时刻（Unix 毫秒） */
  ends_at: number;
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

/**
 * 供应商自定义上游请求头（有序键值对）。
 * 用于除 API Key 外还需额外头的网关；传输层头（Host / Content-Length 等）由后端拒绝，
 * 显式配置的 Authorization 优先于 API Key。
 */
export interface UpstreamHeader {
  key: string;
  value: string;
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
  /** 自定义上游请求头（转发、连通性测试、模型列表三处共用） */
  custom_headers: UpstreamHeader[];
  /** OpenAI 端点拼接时是否带 `/v1`：null/undefined = 自动（URL 结尾已是 /v1 则不补） */
  openai_include_v1?: boolean | null;
  /** Anthropic 端点拼接时是否带 `/v1`（语义同上） */
  anthropic_include_v1?: boolean | null;
  /** 促销活动倒计时（供应商行内展示，见 promotions.ts） */
  promotions: ProviderPromotion[];
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

/** Headroom 本地上下文压缩（服务由「服务」页的 Headroom 服务项托管） */
export interface HeadroomConfig {
  enabled: boolean;
  port: number;
  /** 服务不可用时的策略：failOpen（跳过压缩直接发原始请求，默认）/ failClosed（直接报错） */
  on_unavailable: string;
  /** 关闭文本 ML 压缩（仅结构化压缩，需服务侧同时设 HEADROOM_DISABLE_KOMPRESS=1） */
  disable_kompress: boolean;
  /** 单次压缩调用超时（毫秒） */
  timeout_ms: number;
}

/** Headroom 探活结果 */
export interface HeadroomHealth {
  alive: boolean;
  base_url: string;
  path: string | null;
  status: number | null;
  detail: string;
}

/** 路由链候选：已添加的供应商实例 + 其模型列表中的一个模型（顺序即优先级） */
export interface RouteCandidate {
  provider_id: string;
  model_id: string;
  /**
   * 用户显式声明的「该模型是否支持图片输入」。
   * undefined/null = 自动（交给已确认的纯文本模型注册表判定）。
   * 声明的优先级高于注册表 —— 第三方供应商的视觉模型名字千奇百怪，注册表覆盖不到时
   * 这是唯一可靠的来源。
   */
  supports_image?: boolean | null;
}

/** Codex 官方插件市场状态（后端 `codex_plugin_marketplace_status`）。 */
export interface CodexPluginMarketplaceStatus {
  marketplaceName: string;
  /** 市场内容是否已落盘且清单可解析 */
  installed: boolean;
  /** `~/.codex/config.toml` 里是否已注册本市场 */
  registered: boolean;
  pluginCount: number;
  /** 本市场里已启用的插件数 */
  enabledCount: number;
  root: string;
  configPath: string;
  /**
   * Codex CLI 是否可用。
   * 单个插件的安装/卸载要经 `codex plugin add/remove` 把插件落到客户端缓存，
   * 找不到 CLI 时界面应当直接置灰而不是让用户点了报错。
   */
  cliAvailable: boolean;
}

/** 插件市场里的一个插件（后端 `codex_list_marketplace_plugins`）。 */
export interface CodexPluginInfo {
  name: string;
  /** 清单里声明的分类，界面按它分组 */
  category: string;
  installed: boolean;
  enabled: boolean;
  version?: string | null;
}

/** Claude Code 市场里的一个插件（后端 `claude_list_plugins`）。 */
export interface ClaudePluginInfo {
  /** `插件@市场` —— 安装/卸载都用它寻址 */
  id: string;
  name: string;
  description: string;
  marketplace: string;
  installed: boolean;
  enabled: boolean;
  version?: string | null;
}

/** Claude Code 插件市场状态（后端 `claude_plugin_status`）。 */
export interface ClaudePluginStatus {
  /** 找不到 `claude` CLI 时界面置灰 */
  cliAvailable: boolean;
  /** 已配置的市场名 */
  marketplaces: string[];
  installedCount: number;
  availableCount: number;
  /** 官方市场来源（「添加官方市场」按钮用） */
  officialMarketplaceSource: string;
}

/** 聚合服务配置（本地聚合代理） */
export interface AggregateConfig {
  port: number;
  context_limit: number;
  retry_count: number;
  entry_model: string;
  /** 首字节超时（秒）：连上后迟迟不吐字节就判失败并切下一个候选 */
  first_byte_timeout_secs?: number;
  /** 流式空闲超时（秒）：两个数据块之间的最大间隔 */
  idle_timeout_secs?: number;
}

/** 聚合服务运行状态 */
export interface AggregateStatus {
  running: boolean;
  port: number;
  candidateCount: number;
  detail: string;
}

/** 聚合服务日志行（后端 aggregate-log 事件） */
export interface AggregateLog {
  phase: string;
  line: string;
  level: string;
}

/** 仓库候选视图（路由链页左栏） */
export interface RouteCandidateView {
  provider_id: string;
  provider_name: string;
  provider_category: string;
  model_id: string;
  model_name: string;
  in_chain: boolean;
  /** 链路序号（1 起；未入链为 null） */
  order: number | null;
  /** 指向聚合服务自身（自引用）：入链会造成请求递归，禁止勾选 */
  self_referential: boolean;
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
    /** 纯文本模型预判：按已确认的纯文本注册表，发送前就剥掉图片块 */
    media_heuristic: boolean;
    protocol_mismatch: boolean;
  };
  headroom: HeadroomConfig;
  optimizer: {
    enabled: boolean;
    cache_injection: boolean;
    thinking_optimizer: boolean;
    deepseek_normalize: boolean;
  };
  skills_dir: string;
}

/** 安装 / 卸载 / 升级的结果（后端 `ToolOpResult`）。
 *
 *  此前后端只返回一个字符串，前端靠 `msg.includes("成功")` 猜成败 ——
 *  「已清理 3 处安装文件」这种正常成功文案不含「成功」二字，会被渲染成红色报错。
 *  现在由后端直接给出 ok，前端不再猜。 */
export interface ToolOpResult {
  ok: boolean;
  message: string;
}

export interface DetectedAiTool {
  id: string;
  display_name: string;
  /** 协作模式头像（emoji 或单字符） */
  avatar: string | null;
  /** 协同模式昵称覆盖 */
  nickname: string | null;
  installed: boolean;
  /** 是否由 Kira 声明的包管理器安装（npm/pip 全局注册表可查到）。false 仅作提示，不拦升级/卸载。 */
  pm_managed: boolean;
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
  /** 分叉命令模板（带 {session_id}）：从该会话复制一份新会话再进入；null = 该工具不支持分叉 */
  fork_cmd: string | null;
  cache_dirs: string[];
  category: string;
  supports_openai: boolean;
  supports_anthropic: boolean;
  supports_google: boolean;
  builtin_models: string[];
  supports_optimizer: boolean;
  supports_rectifier: boolean;
  /** 是否使用官方插件市场（前端据此显示「插件市场」区块） */
  supports_plugin_marketplace?: boolean;
  /**
   * 插件市场后端：`codex`（`codex plugin`）或 `claude`（`claude plugin`）。
   * 未声明时按 `codex` 处理（Claude 接入前只有 Codex 系用这个开关）。
   */
  plugin_marketplace_kind?: "codex" | "claude" | null;
  /** MSIX/Store 启动 URI（无普通 exe 时使用） */
  launch_uri: string | null;
  /** 检测到的可执行文件路径（GUI/桌面应用启动用） */
  detected_path: string | null;
  /** 用户在界面上手动指定的路径（空 = 未指定，走注册表默认路径） */
  custom_path?: string | null;
  /** 工具自身的配置文件（有它才支持「设置模型」：模型会写进这个文件） */
  config_file?: { path: string; format: string } | null;
  /** 形态分类（paths.json 的 category：`CLI Code` / `Desktop`） */
  tool_category?: string | null;
  /** 粗粒度归类：`cli` / `desktop` / `other`（列表按它分组） */
  tool_kind?: string | null;
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
  rectifier_media_heuristic: boolean | null;
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
