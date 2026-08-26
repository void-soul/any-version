// 与 src-tauri/src/commands/api/models.rs 对应的前端类型（字段名 snake_case，与 serde 默认一致）。

export interface ApiProject {
  id: string;
  name: string;
  description: string;
  active_env_id: string | null;
  common_headers: KeyValueItem[];
  common_params: KeyValueItem[];
  common_body: KeyValueItem[];
  created_at: string;
  updated_at: string;
}

export interface ApiEnvironment {
  id: string;
  project_id: string;
  name: string;
  variables: Record<string, string | number | boolean | null>;
  sort_order: number;
}

export interface ApiModule {
  id: string;
  project_id: string;
  name: string;
  description: string;
  sort_order: number;
}

// 键值对（Header / 查询参数 / Cookie 共用），支持描述
export interface KeyValueItem {
  key: string;
  value: string;
  enabled: boolean;
  description?: string;
  /** 是否继承自项目通用模板（接口内只读，随模板改名/改值同步） */
  from_template?: boolean;
}

// form-data / urlencoded 条目（text | file）
export interface FormDataItem {
  key: string;
  value: string;
  enabled: boolean;
  kind: "text" | "file";
  file_path: string;
  description?: string;
  /** 是否继承自项目通用 Body 模板（接口内只读） */
  from_template?: boolean;
}

// 认证
export interface Authorization {
  type: "none" | "basic" | "bearer" | "jwt" | "apiKey";
  username: string;
  password: string;
  token: string;
  jwt_token: string;
  api_key_in: "header" | "query";
  api_key_name: string;
  api_key_value: string;
}

// 请求设置
export interface RequestSettings {
  http_version: "auto" | "http1" | "http2";
  verify_ssl: boolean;
  follow_redirects: boolean;
  follow_original_method: boolean;
  follow_authorization_header: boolean;
  remove_referer_on_redirect: boolean;
  strict_http_parser: boolean;
}

export interface ApiEndpoint {
  id: string;
  project_id: string;
  module_id: string | null;
  name: string;
  method: string;
  url: string;
  headers: KeyValueItem[];
  query_params: KeyValueItem[];
  path_params: KeyValueItem[];
  body: string;
  body_type: string;
  body_form: FormDataItem[];
  body_urlencoded: KeyValueItem[];
  body_graphql_query: string;
  body_graphql_variables: string;
  authorization: Authorization;
  cookies: KeyValueItem[];
  settings: RequestSettings;
  response_comment: string;
  is_favorite: boolean;
  description: string;
  docs_md: string;
  timeout_ms: number;
  created_at: string;
  updated_at: string;
}

export interface SendRequestInput {
  method: string;
  url: string;
  headers: KeyValueItem[];
  query_params: KeyValueItem[];
  path_params: KeyValueItem[];
  body: string;
  body_type: string;
  body_form: FormDataItem[];
  body_urlencoded: KeyValueItem[];
  body_graphql_query: string;
  body_graphql_variables: string;
  authorization: Authorization;
  cookies: KeyValueItem[];
  settings: RequestSettings;
  timeout_ms: number;
  variables: Record<string, string | number | boolean | null>;
}

export interface SendRequestOutput {
  ok: boolean;
  status: number;
  status_text: string;
  headers: KeyValueItem[];
  body: string;
  body_truncated: boolean;
  time_ms: number;
  size_bytes: number;
}

// 预设 Headers（项目级）
export interface PresetHeaderSet {
  id: string;
  project_id: string;
  name: string;
  headers: KeyValueItem[];
  created_at: string;
}

// 请求历史
export interface ApiHistoryEntry {
  id: string;
  project_id: string;
  endpoint_id: string | null;
  name: string;
  method: string;
  url: string;
  input: SendRequestInput;
  created_at: string;
}

export interface UnitTestAssertion {
  type: string;
  path?: string | null;
  op?: string | null;
  expected?: string | number | boolean | null;
}

export interface UnitTest {
  id: string;
  endpoint_id: string;
  name: string;
  assertions: UnitTestAssertion[];
  created_at: string;
}

export interface AssertionResult {
  pass: boolean;
  assertion: string;
  actual: string;
}

export interface UnitTestRunOutput {
  pass: boolean;
  time_ms: number;
  status: number;
  results: AssertionResult[];
}

export interface LoadTestConfig {
  concurrency: number;
  duration_secs: number;
  ramp_up_secs: number;
  rps_limit: number;
}

export interface TimelineSample {
  t: number;
  qps: number;
  success: number;
  failed: number;
  avg_ms: number;
}

export interface LoadTestReport {
  total: number;
  success: number;
  failed: number;
  error_rate: number;
  qps_avg: number;
  qps_max: number;
  latency_min_ms: number;
  latency_p50_ms: number;
  latency_p90_ms: number;
  latency_p95_ms: number;
  latency_p99_ms: number;
  latency_max_ms: number;
  latency_avg_ms: number;
  status_codes: [number, number][];
  timeline: TimelineSample[];
}

export interface LoadTestRun {
  id: string;
  endpoint_id: string;
  name: string;
  config: LoadTestConfig;
  report: LoadTestReport | null;
  created_at: string;
}

export interface LoadRunStatus {
  running: boolean;
  elapsed_secs: number;
  total: number;
  success: number;
  failed: number;
  qps: number;
  latency_avg_ms: number;
  latency_p95_ms: number;
  report: LoadTestReport | null;
}

export const METHODS = ["GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS", "HEAD"];

// Body 模式（Postman 风格）
export const BODY_TYPES = [
  { value: "none", label: "无" },
  { value: "formdata", label: "form-data" },
  { value: "form", label: "x-www-form-urlencoded" },
  { value: "raw", label: "raw" },
  { value: "binary", label: "binary" },
  { value: "graphql", label: "GraphQL" },
  { value: "json", label: "JSON" },
];

export const AUTH_TYPES: { value: Authorization["type"]; label: string }[] = [
  { value: "none", label: "No Auth" },
  { value: "bearer", label: "Bearer Token" },
  { value: "basic", label: "Basic Auth" },
  { value: "jwt", label: "JWT Token" },
  { value: "apiKey", label: "API Key" },
];

// 常见自动附加请求头（可隐藏）
export const COMMON_AUTO_HEADERS = [
  "content-type",
  "content-length",
  "host",
  "user-agent",
  "accept",
  "accept-encoding",
  "connection",
];

export const ASSERTION_TYPES: { value: string; label: string; ops?: { value: string; label: string }[] }[] = [
  { value: "status_eq", label: "状态码等于", ops: [{ value: "eq", label: "=" }] },
  { value: "status_lt", label: "状态码小于", ops: [{ value: "lt", label: "<" }] },
  { value: "status_gt", label: "状态码大于", ops: [{ value: "gt", label: ">" }] },
  { value: "body_contains", label: "Body 包含", ops: [{ value: "contains", label: "包含" }] },
  { value: "body_not_contains", label: "Body 不包含", ops: [{ value: "not", label: "不包含" }] },
  { value: "json_path", label: "JSON 路径", ops: [
    { value: "eq", label: "=" },
    { value: "ne", label: "≠" },
    { value: "contains", label: "包含" },
    { value: "gt", label: ">" },
    { value: "lt", label: "<" },
  ] },
  { value: "time_lt_ms", label: "耗时小于(ms)", ops: [{ value: "lt", label: "<" }] },
];

// 随机变量提示（插入辅助）
/** 随机变量：token / 简短描述 / 详细格式说明 / 示例值生成器（与后端 render.rs 行为一致） */
export const RANDOM_VARIABLES: { token: string; desc: string; format: string; example: () => string }[] = [
  { token: "{{$guid}}", desc: "UUID v4", format: "生成 RFC 4122 UUID v4（36 位，含连字符）", example: () => (crypto?.randomUUID?.() ?? "1a2b3c4d-5e6f-7a8b-9c0d-1e2f3a4b5c6d") },
  { token: "{{$timestamp}}", desc: "Unix 秒", format: "当前时间的 Unix 时间戳（秒级，10 位）", example: () => String(Math.floor(Date.now() / 1000)) },
  { token: "{{$isoTimestamp}}", desc: "ISO8601 时间", format: "当前时间的 ISO8601 UTC 格式（如 2026-08-26T12:00:00.000Z）", example: () => new Date().toISOString() },
  { token: "{{$randomInt}}", desc: "0-999999 随机整数", format: "0 ~ 999999 之间的随机整数", example: () => String(Math.floor(Math.random() * 1000000)) },
  { token: "{{random:int:1:100}}", desc: "区间随机整数", format: "格式 {{random:int:最小值:最大值}}，含两端（可自定义区间）", example: () => String(1 + Math.floor(Math.random() * 100)) },
  { token: "{{random:string:8}}", desc: "随机字母数字串", format: "格式 {{random:string:长度}}，随机字母数字串（1-64 位，默认 8）", example: () => {
    const chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    return Array.from({ length: 8 }, () => chars[Math.floor(Math.random() * chars.length)]).join("");
  } },
  { token: "{{$randomEmail}}", desc: "随机邮箱", format: "随机用户名 + 随机域名的邮箱地址", example: () => `user${Math.floor(Math.random() * 100000)}@example.com` },
];

// 默认值工厂
export function defaultAuthorization(): Authorization {
  return { type: "none", username: "", password: "", token: "", jwt_token: "", api_key_in: "header", api_key_name: "", api_key_value: "" };
}

export function defaultSettings(): RequestSettings {
  return {
    http_version: "auto",
    verify_ssl: true,
    follow_redirects: true,
    follow_original_method: false,
    follow_authorization_header: false,
    remove_referer_on_redirect: false,
    strict_http_parser: false,
  };
}
