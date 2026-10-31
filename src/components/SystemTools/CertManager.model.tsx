
// ---------------------------------------------------------------------------
// 类型（与后端 commands/cert.rs 对齐）
// ---------------------------------------------------------------------------
export interface Certificate {
  id: string;
  domains: string[];
  email: string;
  ca: string;
  dns_provider: string;
  credential_id: string;
  /** dns / http（cname 需要公网权威 DNS，暂未开放） */
  validation: string;
  http_port: number;
  renew_before_days: number;
  not_before?: string | null;
  not_after?: string | null;
  last_issue_at?: string | null;
  status: string;
  last_error?: string | null;
  /** 域名列表 → 部署方式关联 */
  deploy_links: DeployLink[];
  /** 兼容旧数据 */
  deploy_node_ids: string[];
}


export interface DeployLink {
  node_id: string;
  /** 目标域名（CDN / 负载均衡等节点使用），为空时取证书首个域名 */
  target: string;
}


/** 证书详情（后端 cert_detail 返回）：解析出的展示信息 + 证书目录内 PEM 原文 */
export interface CertDetail {
  subject?: string | null;
  issuer?: string | null;
  serial?: string | null;
  notBefore?: string | null;
  notAfter?: string | null;
  /** 距到期剩余天数（负数表示已过期） */
  daysLeft?: number | null;
  san: string[];
  pems: Record<string, string>;
}


/**
 * `cert_issue_now` 的返回（后端 `IssueResult`）。
 *
 * `renewed` 必须看：lego 在「还没到续期时间」时会**跳过**签发但仍以退出码 0 结束，
 * 后端过去把这种情况当成签发成功，前端刷新后又看不出任何变化 —— 就是「点申请没反应」。
 */
export interface IssueResult {
  cert: Certificate;
  /** 本次是否真的重新签发了证书 */
  renewed: boolean;
  /** 证书剩余有效期（天，负数表示已过期） */
  days_left?: number | null;
  /** Older backends omit automatic deployment results. */
  deployment?: DeploySummary;
}

export function formatIssueResultSummary(result: Pick<IssueResult, "deployment">, fallback: string): string {
  const failed = result.deployment?.fail_count ?? 0;
  return failed > 0 ? `证书已签发，但 ${failed} 个部署失败，请查看部署日志` : fallback;
}


/** 一次部署里单个节点的结果（与后端 DeployOutcome 对齐） */
export interface DeployOutcome {
  node_id: string;
  node_name: string;
  target: string;
  ok: boolean;
  message: string;
}


export interface DeploySummary {
  results: DeployOutcome[];
  ok_count: number;
  fail_count: number;
}


/** 部署记录（后端 DeployRecord，落盘在 certs/deploy_history.json） */
export interface DeployRecord {
  at: string;
  cert_id: string;
  node_id: string;
  node_name: string;
  target: string;
  ok: boolean;
  message: string;
  /** issue | manual | scheduler */
  trigger: string;
}


// ⚠️ 字段名按 Rust **序列化后的**名字写，不是 Rust 的源码字段名。
//
// `cert.rs` 里这两个字段带 `#[serde(rename = "type")]`：
//   Credential.cred_type -> "type"      DeployNode.node_type -> "type"
// 而磁盘上的 credentials.json / deploy_nodes.json 也是这个格式，所以只能改前端，
// 不能摘掉 Rust 的 rename（摘了就读不出已有文件）。
//
// 之前这里写成 cred_type / node_type，读到的永远是 undefined：
// 凭据下拉的类型显示为空；更糟的是编辑节点时 `initial?.node_type || "linux"`
// 恒等于 "linux"，保存会把非 linux 节点**静默改成 linux**。
export interface DeployNode {
  id: string;
  type: string;
  name: string;
  config: Record<string, string>;
  deploy_before_days: number;
  last_deploy_at?: string | null;
  last_deploy_error?: string | null;
  note: string;
}


export interface Credential {
  id: string;
  name: string;
  type: string;
  data: Record<string, string>;
  note: string;
}


export interface SchedulerLogEntry {
  at: string;
  cert_id: string;
  action: string;
  ok: boolean;
  message: string;
}


export interface SchedulerState {
  enabled: boolean;
  interval_minutes: number;
  last_run_at?: string | null;
  next_run_at?: string | null;
  log: SchedulerLogEntry[];
}


/** 外部域名/IP 的证书监控目标（与后端 MonitorTarget 对齐） */
export interface MonitorTarget {
  id: string;
  host: string;
  port: number;
  note: string;
  enabled: boolean;
  notify_days: number[];
  not_after?: string | null;
  days_left?: number | null;
  subject: string;
  issuer: string;
  last_check_at?: string | null;
  last_error?: string | null;
  last_notified_at?: string | null;
}


/** 通知渠道（与后端 NotifyChannel 对齐） */
export interface NotifyChannel {
  id: string;
  type: string;
  name: string;
  config: Record<string, string>;
  enabled: boolean;
  note: string;
}


export type SubTab = "certs" | "nodes" | "creds" | "sched" | "monitors" | "notify" | "receiver";


// 跨组件卸载持久化：切换子 tab / 离开证书页面再回来时，仍保留上次选中的 tab
// 与「申请中」状态，避免申请未结束时误以为可再次点击申请。

export const issuingSet = new Set<string>();


// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------
export function parseKV(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const t = line.trim();
    if (!t || t.startsWith("#")) continue;
    const i = t.indexOf("=");
    if (i > 0) out[t.slice(0, i).trim()] = t.slice(i + 1).trim();
  }
  return out;
}


// ---- 逐字段规格：部署方式 ----
// 注意：部署方式只保存「连接 / 凭据 / 资源」信息，**不保存域名**；
// 域名与部署方式的关联统一在「域名列表」里配置。
export type FieldSpec = {
  key: string;
  label: string;
  type: "text" | "password";
  placeholder?: string;
  textarea?: boolean;
  select?: { value: string; label: string }[];
  defaultValue?: string;
  visibleWhen?: { key: string; value: string };
};

export function makeNodeConfig(nodeType: string, config: Record<string, string> = {}): Record<string, string> {
  // Preserve legacy local post_script without exposing an additional form field.
  const data: Record<string, string> = nodeType === "local" && config.post_script !== undefined
    ? { post_script: config.post_script } : {};
  for (const field of NODE_FIELDS[nodeType] ?? []) {
    data[field.key] = config[field.key] ?? field.defaultValue ?? "";
  }
  return data;
}

export function visibleNodeFields(nodeType: string, config: Record<string, string>): FieldSpec[] {
  return (NODE_FIELDS[nodeType] ?? []).filter((field) =>
    !field.visibleWhen || config[field.visibleWhen.key] === field.visibleWhen.value);
}


export const AUTH_OPTIONS = [
  { value: "key", label: "私钥认证" },
  { value: "password", label: "密码认证（需 sshpass）" },
];


export const NODE_FIELDS: Record<string, FieldSpec[]> = {
  local: [
    { key: "output_dir", label: "本机证书目录", type: "text", placeholder: "/etc/nginx/certificates" },
    { key: "layout", label: "目录布局", type: "text", defaultValue: "cert", select: [
      { value: "cert", label: "按证书" }, { value: "domain", label: "按域名" },
      { value: "both", label: "按证书和域名" }, { value: "flat", label: "直接写入目录" },
    ] },
    { key: "reload_mode", label: "部署后重载", type: "text", defaultValue: "none", select: [
      { value: "none", label: "不重载" }, { value: "command", label: "执行命令" },
      { value: "nginx", label: "重载 Nginx" }, { value: "service", label: "重启服务" },
    ] },
    { key: "reload_command", label: "重载命令", type: "text", textarea: true, visibleWhen: { key: "reload_mode", value: "command" } },
    { key: "nginx_prefix", label: "Nginx 工作目录（可选）", type: "text", visibleWhen: { key: "reload_mode", value: "nginx" } },
    { key: "nginx_conf", label: "Nginx 配置路径（可选）", type: "text", visibleWhen: { key: "reload_mode", value: "nginx" } },
    { key: "service_name", label: "服务名称", type: "text", visibleWhen: { key: "reload_mode", value: "service" } },
  ],
  windows: [
    { key: "url", label: "接收端地址", type: "text", placeholder: "http://127.0.0.1:9000（/push 可省略）" },
    { key: "token", label: "共享 Token", type: "password" },
  ],
  api: [
    { key: "url", label: "接收端地址", type: "text", placeholder: "http://127.0.0.1:9000" },
    { key: "token", label: "共享 Token", type: "password" },
    {
      key: "api_key",
      label: "客户端 API 密钥（仅拉取用）",
      type: "password",
      // 推送只用上面的 Token；这个值只给「取证书的客户端」算 /api/open/getCertificate 的签名用，
      // 接收端留空即关闭拉取接口。此前提示写成“与接收端一致”，容易被当成推送凭据。
      placeholder: "可选：与接收端 api_key 一致，供客户端拉取接口签名；推送不校验",
    },
  ],
  docker: [
    { key: "url", label: "接收端地址", type: "text", placeholder: "http://127.0.0.1:9000" },
    { key: "token", label: "共享 Token", type: "password" },
    {
      key: "container",
      label: "容器类型",
      type: "text",
      select: [
        { value: "nginx", label: "NGINX" },
        { value: "httpd", label: "HTTPD" },
        { value: "openresty", label: "OpenResty" },
        { value: "custom", label: "自定义" },
      ],
    },
  ],
  ssh: [
    { key: "host", label: "主机地址", type: "text", placeholder: "192.168.1.10" },
    { key: "port", label: "SSH 端口", type: "text", placeholder: "22" },
    { key: "user", label: "用户名", type: "text", placeholder: "root" },
    { key: "auth_method", label: "认证方式", type: "text", select: AUTH_OPTIONS },
    { key: "private_key", label: "私钥路径", type: "text", placeholder: "C:\\Users\\me\\.ssh\\id_rsa" },
    { key: "password", label: "密码（仅密码认证）", type: "password" },
    { key: "cert_dir", label: "证书目录", type: "text", placeholder: "/etc/nginx/certificates" },
    { key: "pre_command", label: "前置命令", type: "text", placeholder: "cd /etc/nginx && mkdir -p certificates", textarea: true },
    { key: "post_command", label: "后置命令", type: "text", placeholder: "nginx -s reload", textarea: true },
  ],
  linux: [
    { key: "host", label: "主机地址", type: "text", placeholder: "192.168.1.1" },
    { key: "port", label: "SSH 端口", type: "text", placeholder: "22" },
    { key: "user", label: "用户名", type: "text", placeholder: "root" },
    { key: "cert_path", label: "证书路径", type: "text", placeholder: "/etc/nginx/certs/cert.pem" },
    { key: "key_path", label: "私钥路径", type: "text", placeholder: "/etc/nginx/certs/key.pem" },
    { key: "reload_cmd", label: "重载命令", type: "text", placeholder: "nginx -s reload" },
  ],
  webhook: [
    { key: "url", label: "回调地址", type: "text", placeholder: "https://example.com/cert-hook" },
    { key: "token", label: "回调令牌", type: "password" },
  ],
  baota: [
    { key: "url", label: "面板地址", type: "text", placeholder: "http://1.2.3.4:8888" },
    { key: "api_sk", label: "API 密钥", type: "password", placeholder: "面板设置 → API 接口" },
    {
      key: "scope",
      label: "部署范围",
      type: "text",
      select: [
        { value: "site", label: "网站 SSL（站点名取目标域名）" },
        { value: "panel", label: "面板 SSL" },
      ],
    },
  ],
  one_panel: [
    { key: "url", label: "面板地址", type: "text", placeholder: "http://1.2.3.4:8080" },
    { key: "api_key", label: "API 密钥", type: "password", placeholder: "面板设置 → API 接口" },
    {
      key: "scope",
      label: "部署范围",
      type: "text",
      select: [
        { value: "site", label: "网站 SSL（站点名取目标域名）" },
        { value: "panel", label: "面板 SSL" },
      ],
    },
  ],
  synology: [
    { key: "url", label: "DSM 地址", type: "text", placeholder: "https://192.168.1.2:5001" },
    { key: "username", label: "账号", type: "text" },
    { key: "password", label: "密码", type: "password" },
    { key: "otp_code", label: "双因素验证码（可选）", type: "text" },
    { key: "cert_id", label: "证书 ID（留空使用默认证书）", type: "text" },
    {
      key: "as_default",
      label: "设为默认证书",
      type: "text",
      select: [
        { value: "false", label: "否" },
        { value: "true", label: "是" },
      ],
    },
  ],
  upyun: [
    { key: "username", label: "又拍云账号", type: "text" },
    { key: "password", label: "密码", type: "password" },
  ],
  baidu_cdn: [
    { key: "access_key_id", label: "AccessKey ID", type: "text" },
    { key: "access_key_secret", label: "Secret Access Key", type: "password" },
  ],
  huawei_cdn: [
    { key: "access_key_id", label: "AccessKey ID", type: "text" },
    { key: "access_key_secret", label: "Secret Access Key", type: "password" },
  ],
  volcengine_cdn: [
    { key: "access_key_id", label: "AccessKey ID", type: "text" },
    { key: "access_key_secret", label: "Secret Access Key", type: "password" },
  ],
  doge_cdn: [
    { key: "access_key", label: "AccessKey", type: "text" },
    { key: "secret_key", label: "SecretKey", type: "password" },
  ],
  aliyun_oss: [
    { key: "bucket", label: "Bucket 名称", type: "text" },
    { key: "endpoint", label: "Endpoint", type: "text", placeholder: "oss-cn-hangzhou.aliyuncs.com" },
    { key: "access_key_id", label: "AccessKey ID", type: "text" },
    { key: "access_key_secret", label: "AccessKey Secret", type: "password" },
  ],
  aliyun_fc: [
    { key: "account_id", label: "阿里云账号 ID", type: "text" },
    { key: "region", label: "Region", type: "text", placeholder: "cn-hangzhou" },
    { key: "access_key_id", label: "AccessKey ID", type: "text" },
    { key: "access_key_secret", label: "AccessKey Secret", type: "password" },
  ],
  aliyun_ssl: [
    { key: "access_key_id", label: "AccessKey ID", type: "text" },
    { key: "access_key_secret", label: "AccessKey Secret", type: "password" },
    { key: "cert_name", label: "证书名前缀（可选）", type: "text" },
  ],
  tencent_ssl: [
    { key: "secret_id", label: "SecretId", type: "text" },
    { key: "secret_key", label: "SecretKey", type: "password" },
    { key: "region", label: "Region（可选）", type: "text", placeholder: "ap-guangzhou" },
    { key: "cert_name", label: "证书名前缀（可选）", type: "text" },
  ],
  qiniu_ssl: [
    { key: "access_key", label: "AccessKey", type: "text" },
    { key: "secret_key", label: "SecretKey", type: "password" },
    { key: "cert_name", label: "证书名前缀（可选）", type: "text" },
  ],
  // 旧版类型（历史数据）：保留可编辑，后端会按新版 SSL 列表接口部署
  qiniu: [
    { key: "access_key", label: "AccessKey", type: "text" },
    { key: "secret_key", label: "SecretKey", type: "password" },
    { key: "cert_name", label: "证书名前缀（可选）", type: "text" },
  ],
  aliyun: [
    { key: "access_key_id", label: "AccessKey ID", type: "text" },
    { key: "access_key_secret", label: "AccessKey Secret", type: "password" },
    { key: "cert_name", label: "证书名前缀（可选）", type: "text" },
  ],
};


export const NODE_TYPE_LABELS: Record<string, string> = {
  local: "本机部署",
  windows: "Windows 接收端 / Nginx",
  api: "API 拉取",
  docker: "Docker 容器拉取",
  ssh: "SSH",
  linux: "Linux（旧版路径模式）",
  webhook: "Webhook",
  baota: "宝塔面板",
  one_panel: "1Panel 面板",
  synology: "群晖 DSM",
  upyun: "又拍云",
  baidu_cdn: "百度智能云 CDN",
  huawei_cdn: "华为云 CDN",
  volcengine_cdn: "火山引擎 CDN",
  doge_cdn: "多吉云 CDN",
  aliyun_oss: "阿里云 OSS",
  aliyun_fc: "阿里云函数计算",
  aliyun_ssl: "阿里云 SSL 证书列表",
  tencent_ssl: "腾讯云 SSL 证书列表",
  qiniu_ssl: "七牛云 SSL 证书列表",
  qiniu: "七牛云（旧版）",
  aliyun: "阿里云（旧版）",
};


// ---- DNS 服务商标识 → 显示名称 ----
export const DNS_PROVIDER_MAP = [
  { id: "alidns", label: "阿里云" },
  { id: "route53", label: "AWS" },
  { id: "godaddy", label: "GoDaddy" },
  { id: "cloudflare", label: "CloudFlare" },
  { id: "dnspod", label: "DnsPod（腾讯云）" },
  { id: "huaweicloud", label: "华为云" },
  { id: "baiducloud", label: "百度智能云" },
  { id: "dnsla", label: "DNS.LA" },
  { id: "westcn", label: "西部数码" },
  { id: "volcengine", label: "火山引擎" },
  { id: "tencentdns", label: "腾讯云" },
];


// ---- 逐字段规格：DNS 凭据 ----
// 已有映射的沿用别名（cert.rs lego_env 负责转换）；新增提供商的键名即 lego 环境变量名（后端 fallback 透传）。
export const CRED_FIELDS: Record<string, FieldSpec[]> = {
  alidns: [
    { key: "access_key", label: "AccessKey ID", type: "text" },
    { key: "secret_key", label: "AccessKey Secret", type: "password" },
    { key: "region", label: "Region（可选）", type: "text", placeholder: "cn-hangzhou" },
  ],
  route53: [
    { key: "AWS_ACCESS_KEY_ID", label: "AccessKey ID", type: "text" },
    { key: "AWS_SECRET_ACCESS_KEY", label: "SecretAccessKey", type: "password" },
    { key: "AWS_REGION", label: "Region（可选）", type: "text", placeholder: "us-east-1" },
  ],
  godaddy: [
    { key: "GODADDY_API_KEY", label: "API Key", type: "text" },
    { key: "GODADDY_API_SECRET", label: "API Secret", type: "password" },
  ],
  cloudflare: [
    { key: "api_token", label: "API Token", type: "password" },
  ],
  dnspod: [
    { key: "DNSPOD_API_KEY", label: "API Key（格式：ID,Token）", type: "password" },
  ],
  huaweicloud: [
    { key: "HUAWEICLOUD_ACCESS_KEY_ID", label: "AccessKey ID", type: "text" },
    { key: "HUAWEICLOUD_SECRET_ACCESS_KEY", label: "SecretAccessKey", type: "password" },
    { key: "HUAWEICLOUD_REGION", label: "Region", type: "text", placeholder: "cn-north-4" },
  ],
  baiducloud: [
    { key: "BAIDUCLOUD_ACCESS_KEY_ID", label: "AccessKey ID", type: "text" },
    { key: "BAIDUCLOUD_SECRET_ACCESS_KEY", label: "SecretAccessKey", type: "password" },
  ],
  dnsla: [
    { key: "DNSLA_API_ID", label: "API ID", type: "text" },
    { key: "DNSLA_API_SECRET", label: "API Secret", type: "password" },
  ],
  westcn: [
    { key: "WESTCN_USERNAME", label: "用户名", type: "text" },
    { key: "WESTCN_API_PASSWORD", label: "API 密码", type: "password" },
  ],
  volcengine: [
    { key: "VOLCENGINE_ACCESS_KEY", label: "AccessKey", type: "text" },
    { key: "VOLCENGINE_SECRET_KEY", label: "SecretKey", type: "password" },
  ],
  tencentdns: [
    { key: "secret_id", label: "SecretId", type: "text" },
    { key: "secret_key", label: "SecretKey", type: "password" },
  ],
};


export function fmtDate(s?: string | null): string {
  if (!s) return "-";
  return s.replace("T", " ").slice(0, 19);
}


/** 申请日志事件（与后端 cert.rs `CERT_ISSUE_LOG_EVENT` 对齐） */
export const CERT_ISSUE_LOG_EVENT = "cert-issue-log";


export interface IssueLogPayload {
  cert_id: string;
  line: string;
}


/** 距某时间点还有多少天（向下取整，负数表示已过期）；无法解析返回 null。 */
export function daysUntil(s?: string | null): number | null {
  if (!s) return null;
  const t = Date.parse(s);
  if (Number.isNaN(t)) return null;
  return Math.floor((t - Date.now()) / 86_400_000);
}


// ---- 通知渠道 ----
export const NOTIFY_TYPES = [
  { value: "dingtalk", label: "钉钉机器人" },
  { value: "wecom", label: "企业微信机器人" },
  { value: "slack", label: "Slack" },
  { value: "discord", label: "Discord" },
  { value: "teams", label: "Microsoft Teams" },
  { value: "webhook", label: "自定义 Webhook" },
];
