import { useState, useEffect, useCallback, useRef } from "react";
import { theamedAlert, alertError, theamedConfirm } from "../shared/ThemedAlert";
import { toast } from "../shared/Toast";
import { SharedModal } from "../shared/Modal";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  ShieldCheck,
  Server,
  KeyRound,
  History,
  Plus,
  Trash2,
  RefreshCw,
  Play,
  Cable,
  Download,
  Copy,
  Check,
  Activity,
  Bell,
} from "lucide-react";

// ---------------------------------------------------------------------------
// 类型（与后端 commands/cert.rs 对齐）
// ---------------------------------------------------------------------------
interface Certificate {
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

interface DeployLink {
  node_id: string;
  /** 目标域名（CDN / 负载均衡等节点使用），为空时取证书首个域名 */
  target: string;
}

/** 证书详情（后端 cert_detail 返回）：解析出的展示信息 + 证书目录内 PEM 原文 */
interface CertDetail {
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
interface IssueResult {
  cert: Certificate;
  /** 本次是否真的重新签发了证书 */
  renewed: boolean;
  /** 证书剩余有效期（天，负数表示已过期） */
  days_left?: number | null;
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
interface DeployNode {
  id: string;
  type: string;
  name: string;
  config: Record<string, string>;
  deploy_before_days: number;
  last_deploy_at?: string | null;
  last_deploy_error?: string | null;
  note: string;
}

interface Credential {
  id: string;
  name: string;
  type: string;
  data: Record<string, string>;
  note: string;
}

interface SchedulerLogEntry {
  at: string;
  cert_id: string;
  action: string;
  ok: boolean;
  message: string;
}

interface SchedulerState {
  enabled: boolean;
  interval_minutes: number;
  last_run_at?: string | null;
  next_run_at?: string | null;
  log: SchedulerLogEntry[];
}

/** 外部域名/IP 的证书监控目标（与后端 MonitorTarget 对齐） */
interface MonitorTarget {
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
interface NotifyChannel {
  id: string;
  type: string;
  name: string;
  config: Record<string, string>;
  enabled: boolean;
  note: string;
}

type SubTab = "certs" | "nodes" | "creds" | "sched" | "monitors" | "notify";

// 跨组件卸载持久化：切换子 tab / 离开证书页面再回来时，仍保留上次选中的 tab
// 与「申请中」状态，避免申请未结束时误以为可再次点击申请。
let persistedTab: SubTab = "certs";
const issuingSet = new Set<string>();

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------
function parseKV(text: string): Record<string, string> {
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
type FieldSpec = {
  key: string;
  label: string;
  type: "text" | "password";
  placeholder?: string;
  textarea?: boolean;
  select?: { value: string; label: string }[];
};

const AUTH_OPTIONS = [
  { value: "key", label: "私钥认证" },
  { value: "password", label: "密码认证（需 sshpass）" },
];

const NODE_FIELDS: Record<string, FieldSpec[]> = {
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

const NODE_TYPE_LABELS: Record<string, string> = {
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
const DNS_PROVIDER_MAP = [
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
const CRED_FIELDS: Record<string, FieldSpec[]> = {
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

function fmtDate(s?: string | null): string {
  if (!s) return "-";
  return s.replace("T", " ").slice(0, 19);
}

/** 申请日志事件（与后端 cert.rs `CERT_ISSUE_LOG_EVENT` 对齐） */
const CERT_ISSUE_LOG_EVENT = "cert-issue-log";

interface IssueLogPayload {
  cert_id: string;
  line: string;
}

/** 距某时间点还有多少天（向下取整，负数表示已过期）；无法解析返回 null。 */
function daysUntil(s?: string | null): number | null {
  if (!s) return null;
  const t = Date.parse(s);
  if (Number.isNaN(t)) return null;
  return Math.floor((t - Date.now()) / 86_400_000);
}

/**
 * 申请日志弹窗：live 模式逐行追加并自动滚到底，历史模式直接展示落盘内容。
 *
 * 两种模式共用同一个组件，是为了让「正在跑」和「回看」看到的东西完全一致 ——
 * 申请途中关掉面板，再点行内「日志」看到的就是同一份内容（后端同时落盘）。
 */
function IssueLogModal({
  open,
  title,
  lines,
  summary,
  onClose,
}: {
  open: boolean;
  title: string;
  lines: string[];
  summary?: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const boxRef = useRef<HTMLDivElement | null>(null);
  const [copied, setCopied] = useState(false);

  // 新行进来就贴底：签发过程几十秒，用户不该边看边手动滚
  useEffect(() => {
    const el = boxRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines.length, open]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // 复制失败不影响查看
    }
  };

  return (
    <SharedModal
      open={open}
      onClose={onClose}
      title={title}
      width={780}
      footer={
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={copy}
            className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption cursor-pointer"
          >
            {copied ? t("certmgr.copied") : t("certmgr.copy")}
          </button>
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold cursor-pointer"
          >
            {t("certmgr.close")}
          </button>
        </div>
      }
    >
      {summary && <div className="mb-2 text-caption text-slate-300">{summary}</div>}
      <div
        ref={boxRef}
        className="max-h-[55vh] overflow-y-auto rounded-ctl border border-white/10 bg-black/40 p-2"
      >
        {lines.length === 0 ? (
          <div className="text-caption text-slate-500">{t("certmgr.issueLogEmpty")}</div>
        ) : (
          <pre className="text-micro font-mono whitespace-pre-wrap break-all text-slate-300">
            {lines.join("\n")}
          </pre>
        )}
      </div>
    </SharedModal>
  );
}

// ---------------------------------------------------------------------------
// 组件
// ---------------------------------------------------------------------------
export default function CertManager() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<SubTab>(persistedTab);
  const changeTab = (k: SubTab) => {
    persistedTab = k;
    setTab(k);
  };

  return (
    <div className="flex-1 overflow-hidden flex min-h-0 select-none">
      <div className="w-40 flex-shrink-0 border-r border-white/5 py-3 px-2 space-y-0.5 overflow-y-auto">
        {([
          { k: "certs" as const, label: t("certmgr.tabCerts"), icon: ShieldCheck },
          { k: "nodes" as const, label: t("certmgr.tabNodes"), icon: Server },
          { k: "creds" as const, label: t("certmgr.tabCreds"), icon: KeyRound },
          { k: "monitors" as const, label: t("certmgr.tabMonitors"), icon: Activity },
          { k: "notify" as const, label: t("certmgr.tabNotify"), icon: Bell },
          { k: "sched" as const, label: t("certmgr.tabSched"), icon: History },
        ]).map(({ k, label, icon: Icon }) => (
          <button
            key={k}
            onClick={() => changeTab(k)}
            className={`w-full px-3 py-2 rounded-ctl text-caption font-semibold flex items-center gap-2 transition-all cursor-pointer text-left ${
              tab === k
                ? "bg-[var(--module-accent)] text-white shadow-md shadow-[var(--module-accent-ring)]"
                : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
            }`}
          >
            <Icon className="w-3.5 h-3.5 flex-shrink-0" />
            {label}
          </button>
        ))}
      </div>
      <div className="flex-1 min-h-0 overflow-y-auto px-6 py-4">
        {tab === "certs" && <CertList />}
        {tab === "nodes" && <DeployNodes />}
        {tab === "creds" && <Credentials />}
        {tab === "monitors" && <MonitorView />}
        {tab === "notify" && <NotifyView />}
        {tab === "sched" && <SchedulerView />}
      </div>
    </div>
  );
}

// ---- 证书列表 ----
function CertList() {
  const { t } = useTranslation();
  const [certs, setCerts] = useState<Certificate[]>([]);
  const [creds, setCreds] = useState<Credential[]>([]);
  const [nodes, setNodes] = useState<DeployNode[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [editing, setEditing] = useState<Certificate | null>(null);
  const [issuing, setIssuing] = useState<Set<string>>(() => new Set(issuingSet));
  const markIssuing = (id: string, on: boolean) => {
    if (on) issuingSet.add(id); else issuingSet.delete(id);
    setIssuing(new Set(issuingSet));
  };
  // 「查看证书」弹窗：详情 + PEM 原文 + 下载证书包
  const [detail, setDetail] = useState<CertDetail | null>(null);
  const [detailId, setDetailId] = useState<string | null>(null);
  const [copiedName, setCopiedName] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);
  // 申请日志：live 是正在跑的那一次（实时流），history 是手动打开的历史
  const [liveLog, setLiveLog] = useState<{
    id: string;
    domain: string;
    lines: string[];
    summary?: string;
  } | null>(null);
  const [historyLog, setHistoryLog] = useState<{ domain: string; lines: string[] } | null>(null);
  // 日志面板是否还开着：用户中途关掉后失败才需要额外弹一个阻塞提示
  const liveLogOpenRef = useRef(false);

  const refresh = useCallback(async () => {
    setCerts(await invoke<Certificate[]>("cert_list"));
    setCreds(await invoke<Credential[]>("credential_list"));
    setNodes(await invoke<DeployNode[]>("deploy_node_list"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const issue = async (id: string) => {
    if (issuing.has(id)) return;
    const target = certs.find((c) => c.id === id);
    const threshold = target?.renew_before_days ?? 30;
    const left = daysUntil(target?.not_after);
    // 手动「申请」= 强制重签。证书还没到续期阈值时先说清代价：
    // 同一组域名对 Let's Encrypt 每周只有 5 次重复签发额度，白跑几次当天就签不出来了。
    if (target?.status === "issued" && left !== null && left > threshold) {
      const ok = await theamedConfirm(
        t("certmgr.forceRenewConfirm", { days: left, threshold }),
        { danger: true, confirmText: t("certmgr.apply") },
      );
      if (!ok) return;
    }
    const domain = target?.domains.join(", ") || id;
    markIssuing(id, true);
    liveLogOpenRef.current = true;
    setLiveLog({ id, domain, lines: [] });
    let unlisten: (() => void) | undefined;
    try {
      // 先订阅再 invoke：否则 lego 最早的几行（命令行、账号复用）会在订阅前就推完
      unlisten = await listen<IssueLogPayload>(CERT_ISSUE_LOG_EVENT, (e) => {
        if (e.payload.cert_id !== id) return;
        const line = e.payload.line;
        setLiveLog((prev) =>
          prev && prev.id === id ? { ...prev, lines: [...prev.lines, line] } : prev,
        );
      });
      const res = await invoke<IssueResult>("cert_issue_now", { id });
      await refresh();
      if (res.renewed) {
        const summary = t("certmgr.applyDone", { time: fmtDate(res.cert?.not_after) });
        toast(summary);
        setLiveLog((p) => (p && p.id === id ? { ...p, summary } : p));
      } else {
        // 后端如实回报「没签」：说清楚原因，别让用户以为按钮坏了
        const summary = t("certmgr.applySkipped", { days: res.days_left ?? 0, threshold });
        setLiveLog((p) => (p && p.id === id ? { ...p, summary } : p));
      }
    } catch (e) {
      const msg = String(e);
      setLiveLog((p) =>
        p && p.id === id
          ? { ...p, lines: [...p.lines, "", `✗ ${msg}`], summary: t("certmgr.applyFail", { err: msg }) }
          : p,
      );
      // 日志面板还开着就已经把错误摆在最上面了；只有用户中途关掉时才额外弹提示
      if (!liveLogOpenRef.current) alertError(t("certmgr.applyFail", { err: msg }));
    } finally {
      unlisten?.();
      markIssuing(id, false);
    }
  };

  const closeLiveLog = () => {
    liveLogOpenRef.current = false;
    setLiveLog(null);
  };

  /** 打开某证书的历史申请日志（后端落盘在 certs/logs/<id>.log） */
  const openLog = async (c: Certificate) => {
    const domain = c.domains.join(", ") || c.id;
    try {
      const text = await invoke<string>("cert_issue_log", { id: c.id });
      setHistoryLog({ domain, lines: text ? text.replace(/\n+$/, "").split("\n") : [] });
    } catch (e) {
      theamedAlert(String(e), "err");
    }
  };
  const del = async (id: string) => {
    if (!confirm(t("certmgr.delCertConfirm"))) return;
    await invoke("cert_delete", { id });
    refresh();
  };
  const viewDetail = async (id: string) => {
    try {
      setDetailId(id);
      setDetail(await invoke<CertDetail>("cert_detail", { id }));
    } catch (e) {
      setDetailId(null);
      theamedAlert(String(e));
    }
  };
  const closeDetail = () => {
    setDetail(null);
    setDetailId(null);
    setCopiedName(null);
  };
  const copyPem = async (name: string, content: string) => {
    try {
      await navigator.clipboard.writeText(content);
      setCopiedName(name);
      setTimeout(() => setCopiedName(null), 1500);
    } catch {
      // 复制失败不打断查看
    }
  };
  // 下载证书包：通用命名 4 文件 + lego 原生文件（后端打包为 zip）
  const downloadZip = async () => {
    if (!detailId) return;
    const domain = certs.find((c) => c.id === detailId)?.domains[0] ?? detailId;
    const safe = domain.replace(/[^A-Za-z0-9._-]/g, "_");
    try {
      const filePath = await saveDialog({
        title: t("certmgr.downloadTitle"),
        defaultPath: `${safe}.zip`,
        filters: [{ name: "ZIP", extensions: ["zip"] }],
      });
      if (!filePath || typeof filePath !== "string") return;
      setDownloading(true);
      const saved = await invoke<string>("cert_export_zip", {
        id: detailId,
        targetPath: filePath,
      });
      theamedAlert(t("certmgr.downloadDone", { path: saved }));
    } catch (e) {
      alertError(t("certmgr.downloadFail", { err: String(e) }));
    } finally {
      setDownloading(false);
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-bold text-slate-200">{t("certmgr.certsTitle")}</h3>
        <button
          onClick={() => setShowForm((v) => !v)}
          className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5"
        >
          <Plus className="w-3.5 h-3.5" /> {t("certmgr.newCert")}
        </button>
      </div>

      {showForm && (
        <CertForm
          creds={creds}
          nodes={nodes}
          onDone={() => {
            setShowForm(false);
            refresh();
          }}
        />
      )}
      {editing && (
        <CertForm
          key={editing.id}
          creds={creds}
          nodes={nodes}
          initial={editing}
          onDone={() => {
            setEditing(null);
            refresh();
          }}
        />
      )}

      <div className="overflow-x-auto rounded-ctl border border-white/5">
        <table className="w-full text-caption text-slate-300">
          <thead className="bg-white/5 text-slate-400">
            <tr>
              <th className="px-3 py-2 text-left">{t("certmgr.colDomain")}</th>
              <th className="px-3 py-2 text-left">CA</th>
              <th className="px-3 py-2 text-left">{t("certmgr.colDns")}</th>
              <th className="px-3 py-2 text-left">{t("certmgr.validation")}</th>
              <th className="px-3 py-2 text-left">{t("certmgr.colExpire")}</th>
              <th className="px-3 py-2 text-left">{t("certmgr.colStatus")}</th>
              <th className="px-3 py-2 text-left">{t("certmgr.colNodes")}</th>
              <th className="px-3 py-2 text-right">{t("certmgr.colOps")}</th>
            </tr>
          </thead>
          <tbody>
            {certs.map((c) => (
              <tr key={c.id} className="border-t border-white/5">
                <td className="px-3 py-2">{c.domains.join(", ")}</td>
                <td className="px-3 py-2">{c.ca}</td>
                <td className="px-3 py-2">{c.dns_provider || "-"}</td>
                <td className="px-3 py-2">
                  {c.validation === "http"
                    ? t("certmgr.validationHttp")
                    : t("certmgr.validationDns")}
                </td>
                <td className="px-3 py-2">{fmtDate(c.not_after)}</td>
                <td className="px-3 py-2">
                  <span
                    title={c.last_error || undefined}
                    className={
                      c.status === "issued"
                        ? "text-emerald-400"
                        : c.status === "error"
                          ? "text-rose-400"
                          : "text-amber-400"
                    }
                  >
                    {c.status}
                  </span>
                </td>
                <td className="px-3 py-2">
                  {c.deploy_links?.length ? (
                    <span
                      className="text-tiny text-slate-300"
                      title={c.deploy_links
                        .map((l) => {
                          const n = nodes.find((x) => x.id === l.node_id);
                          return `${n?.name ?? l.node_id}${l.target ? ` → ${l.target}` : ""}`;
                        })
                        .join("\n")}
                    >
                      {c.deploy_links
                        .map((l) => {
                          const n = nodes.find((x) => x.id === l.node_id);
                          return `${n?.name ?? l.node_id}${l.target ? `→${l.target}` : ""}`;
                        })
                        .join(" · ")}
                    </span>
                  ) : (
                    <span className="text-slate-500">{t("certmgr.nodesCount", { count: 0 })}</span>
                  )}
                </td>
                <td className="px-3 py-2 text-right whitespace-nowrap">
                  <button onClick={() => issue(c.id)} disabled={issuing.has(c.id)} className="text-[var(--module-accent)] hover:text-[var(--module-accent-strong)] mr-2 disabled:opacity-50">
                    {issuing.has(c.id) ? t("certmgr.issuing") : t("certmgr.apply")}
                  </button>
                  <button
                    onClick={() => { setEditing(c); setShowForm(false); }}
                    className="text-slate-300 hover:text-white mr-2"
                  >
                    {t("certmgr.edit")}
                  </button>
                  <button
                    onClick={() => viewDetail(c.id)}
                    disabled={c.status !== "issued"}
                    title={c.status === "issued" ? t("certmgr.view") : t("certmgr.notIssued")}
                    className="text-sky-400 hover:text-sky-300 mr-2 disabled:opacity-40 disabled:cursor-not-allowed"
                  >
                    {t("certmgr.view")}
                  </button>
                  <button
                    onClick={() => openLog(c)}
                    className="text-slate-400 hover:text-slate-200 mr-2"
                    title={t("certmgr.issueLogHint")}
                  >
                    {t("certmgr.log")}
                  </button>
                  <button onClick={() => del(c.id)} className="text-rose-400 hover:text-rose-300">
                    <Trash2 className="w-3.5 h-3.5 inline" />
                  </button>
                </td>
              </tr>
            ))}
            {certs.length === 0 && (
              <tr>
                <td colSpan={8} className="px-3 py-6 text-center text-slate-500">{t("certmgr.certsEmpty")}</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {detail && (
        <div className="fixed inset-0 modal-mask bg-black/60 flex items-center justify-center z-50">
          {/* 面板用 ui-modal（实色背景）：之前误用了 ui-input，5% 白底半透明，后面的列表会透上来 */}
          <div
            className="ui-modal w-[70vw] max-h-[85vh] flex flex-col overflow-hidden"
            onClick={(e) => e.stopPropagation()}
          >
            {/* 头部：标题 + 下载 + 关闭 */}
            <div className="flex items-center gap-2 px-4 py-3 border-b border-white/10">
              <ShieldCheck className="w-4 h-4 text-[var(--module-accent)] flex-shrink-0" />
              <span className="text-body font-bold text-slate-200 flex-shrink-0">
                {t("certmgr.detailTitle")}
              </span>
              <span className="text-caption text-slate-400 truncate">
                {certs.find((c) => c.id === detailId)?.domains.join(", ")}
              </span>
              <div className="flex-1" />
              <button
                onClick={downloadZip}
                disabled={downloading}
                className="px-2.5 py-1.5 rounded-md bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5 disabled:opacity-50 cursor-pointer"
              >
                <Download className="w-3.5 h-3.5" />
                {downloading ? t("certmgr.downloading") : t("certmgr.download")}
              </button>
              <button
                onClick={closeDetail}
                className="text-caption text-slate-400 hover:text-slate-200 cursor-pointer"
              >
                {t("certmgr.close")}
              </button>
            </div>

            <div className="flex-1 min-h-0 overflow-y-auto px-4 py-3 space-y-3">
              {/* 证书信息 */}
              <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-caption">
                <span className="text-slate-500">{t("certmgr.detailSubject")}</span>
                <span className="text-slate-200 break-all">{detail.subject ?? "—"}</span>
                <span className="text-slate-500">{t("certmgr.detailIssuer")}</span>
                <span className="text-slate-200 break-all">{detail.issuer ?? "—"}</span>
                <span className="text-slate-500">{t("certmgr.detailSerial")}</span>
                <span className="text-slate-200 font-mono break-all">{detail.serial ?? "—"}</span>
                <span className="text-slate-500">{t("certmgr.detailValidity")}</span>
                <span className="text-slate-200">
                  {fmtDate(detail.notBefore)} → {fmtDate(detail.notAfter)}
                  {typeof detail.daysLeft === "number" && (
                    <span
                      className={`ml-2 ${
                        detail.daysLeft < 0
                          ? "text-rose-400"
                          : detail.daysLeft <= 15
                            ? "text-amber-400"
                            : "text-emerald-400"
                      }`}
                    >
                      {detail.daysLeft < 0
                        ? t("certmgr.detailExpired", { days: Math.abs(detail.daysLeft) })
                        : t("certmgr.detailDaysLeft", { days: detail.daysLeft })}
                    </span>
                  )}
                </span>
                <span className="text-slate-500">{t("certmgr.detailSan")}</span>
                <span className="text-slate-200 break-all">
                  {detail.san.length > 0 ? detail.san.join(", ") : "—"}
                </span>
              </div>

              {/* PEM 原文（可逐段复制） */}
              <div className="flex items-center gap-2 pt-1">
                <span className="text-caption font-bold text-slate-300">{t("certmgr.pemTitle")}</span>
                <span className="text-tiny text-slate-500">{t("certmgr.pemHint")}</span>
              </div>
              {Object.keys(detail.pems).length === 0 && (
                <div className="text-caption text-slate-500">{t("certmgr.pemEmpty")}</div>
              )}
              {Object.entries(detail.pems).map(([name, content]) => (
                <div
                  key={name}
                  className="rounded-ctl border border-white/10 bg-black/20 overflow-hidden"
                >
                  <div className="flex items-center gap-2 px-2.5 py-1.5 bg-white/5">
                    <span className="text-tiny text-slate-300 font-mono">{name}</span>
                    <div className="flex-1" />
                    <button
                      onClick={() => copyPem(name, content)}
                      className="text-tiny text-slate-400 hover:text-slate-200 flex items-center gap-1 cursor-pointer"
                    >
                      {copiedName === name ? (
                        <>
                          <Check className="w-3 h-3 text-emerald-400" />
                          {t("certmgr.copied")}
                        </>
                      ) : (
                        <>
                          <Copy className="w-3 h-3" />
                          {t("certmgr.copy")}
                        </>
                      )}
                    </button>
                  </div>
                  <pre className="text-micro p-2 overflow-auto max-h-32 whitespace-pre-wrap text-slate-400">
                    {content}
                  </pre>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}

      {/* 申请过程实时日志（关掉面板不影响后端继续跑，内容同时落盘） */}
      <IssueLogModal
        open={!!liveLog}
        title={t("certmgr.issueLogTitle", { domain: liveLog?.domain ?? "" })}
        lines={liveLog?.lines ?? []}
        summary={liveLog ? liveLog.summary ?? t("certmgr.issueLogRunning") : undefined}
        onClose={closeLiveLog}
      />
      {/* 历史日志（certs/logs/<证书>.log） */}
      <IssueLogModal
        open={!!historyLog}
        title={t("certmgr.issueLogTitle", { domain: historyLog?.domain ?? "" })}
        lines={historyLog?.lines ?? []}
        onClose={() => setHistoryLog(null)}
      />
    </div>
  );
}

function CertForm({
  creds,
  nodes,
  initial,
  onDone,
}: {
  creds: Credential[];
  nodes: DeployNode[];
  initial?: Certificate | null;
  onDone: () => void;
}) {
  const { t } = useTranslation();
  const [domains, setDomains] = useState(initial?.domains.join(", ") ?? "");
  const [email, setEmail] = useState(initial?.email ?? "");
  const [ca, setCa] = useState(initial?.ca ?? "letsencrypt");
  const [validation, setValidation] = useState(initial?.validation ?? "dns");
  const [httpPort, setHttpPort] = useState(initial?.http_port ?? 80);
  const [dnsProvider, setDnsProvider] = useState(initial?.dns_provider ?? "alidns");
  const [credentialId, setCredentialId] = useState(initial?.credential_id ?? "");
  const [renewBefore, setRenewBefore] = useState(initial?.renew_before_days ?? 30);
  const [links, setLinks] = useState<Record<string, string>>(() => {
    const src = initial?.deploy_links?.length
      ? initial.deploy_links
      : (initial?.deploy_node_ids ?? []).map((node_id) => ({ node_id, target: "" }));
    return Object.fromEntries(src.map((l) => [l.node_id, l.target]));
  });
  const [err, setErr] = useState("");

  const submit = async () => {
    setErr("");
    try {
      const domList = domains
        .split(/[,\s]+/)
        .map((d) => d.trim())
        .filter(Boolean);
      // 通配符自动补根域
      const wild = domList.find((d) => d.startsWith("*."));
      if (wild) {
        const root = wild.slice(2);
        if (!domList.includes(root)) domList.push(root);
      }
      if (domList.length === 0) return setErr(t("certmgr.domainRequired"));
      if (validation === "http" && domList.some((d) => d.startsWith("*."))) {
        return setErr(t("certmgr.httpWildcard"));
      }
      if (validation === "dns" && !credentialId) {
        return setErr(t("certmgr.credRequired"));
      }
      const deployLinks: DeployLink[] = nodes
        .filter((n) => links[n.id] !== undefined)
        .map((n) => ({ node_id: n.id, target: links[n.id] || "" }));

      if (initial) {
        await invoke("cert_update", {
          id: initial.id,
          domains: domList,
          email,
          ca,
          dnsProvider: validation === "dns" ? dnsProvider : "",
          credentialId: validation === "dns" ? credentialId : "",
          validation,
          httpPort,
          renewBeforeDays: renewBefore,
        });
        await invoke("cert_update_links", { id: initial.id, deployLinks });
      } else {
        await invoke("cert_create", {
          domains: domList,
          email,
          ca,
          dnsProvider: validation === "dns" ? dnsProvider : "",
          credentialId: validation === "dns" ? credentialId : "",
          validation,
          httpPort,
          renewBeforeDays: renewBefore,
          deployLinks,
        });
      }
      onDone();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="rounded-ctl border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.domainLabel")}
          <input value={domains} onChange={(e) => setDomains(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="*.example.com, example.com" />
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.emailLabel")}
          <input value={email} onChange={(e) => setEmail(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="admin@example.com" />
        </label>
        <label className="text-caption text-slate-400">
          CA
          <select value={ca} onChange={(e) => setCa(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            <option value="letsencrypt" className="bg-slate-800 text-slate-200">{t("certmgr.letsencrypt")}</option>
            <option value="letsencrypt-staging" className="bg-slate-800 text-slate-200">{t("certmgr.letsencryptStaging")}</option>
          </select>
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.validation")}
          <select value={validation} onChange={(e) => setValidation(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            <option value="dns" className="bg-slate-800 text-slate-200">{t("certmgr.validationDns")}</option>
            <option value="http" className="bg-slate-800 text-slate-200">{t("certmgr.validationHttp")}</option>
            <option value="cname" disabled className="bg-slate-800 text-slate-500">{t("certmgr.validationCname")}</option>
          </select>
        </label>
        {validation === "http" && (
          <label className="text-caption text-slate-400">
            {t("certmgr.httpPort")}
            <input type="number" value={httpPort} onChange={(e) => setHttpPort(Number(e.target.value))} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
          </label>
        )}
        {validation === "dns" && (
          <>
            <label className="text-caption text-slate-400">
              {t("certmgr.dnsProvider")}
              <select value={dnsProvider} onChange={(e) => setDnsProvider(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
                {[...DNS_PROVIDER_MAP, { id: "custom", label: t("certmgr.custom") }].map((p) => (
                  <option key={p.id} value={p.id} className="bg-slate-800 text-slate-200">{t(p.label)}</option>
                ))}
              </select>
            </label>
            <label className="text-caption text-slate-400">
              {t("certmgr.credLabel")}
              <select value={credentialId} onChange={(e) => setCredentialId(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
                <option value="">{t("certmgr.select")}</option>
                {creds.map((c) => (
                  <option key={c.id} value={c.id}>{c.name}（{c.type}）</option>
                ))}
              </select>
            </label>
          </>
        )}
        <label className="text-caption text-slate-400">
          {t("certmgr.renewDays")}
          <input type="number" value={renewBefore} onChange={(e) => setRenewBefore(Number(e.target.value))} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>

      {validation === "http" && (
        <p className="text-tiny text-amber-400/90 bg-amber-500/5 border border-amber-500/10 rounded-ctl p-2">
          {t("certmgr.httpHint")}
        </p>
      )}

      <div className="text-caption text-slate-400">
        {t("certmgr.deployNodes")}
        <p className="mt-0.5 text-tiny text-slate-500">{t("certmgr.deployLinkHint")}</p>
        <div className="mt-2 space-y-1.5">
          {nodes.map((n) => {
            const checked = links[n.id] !== undefined;
            return (
              <div key={n.id} className="flex items-center gap-2 text-tiny text-slate-300 bg-black/30 rounded px-2 py-1.5">
                <label className="flex items-center gap-1.5 cursor-pointer shrink-0">
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={(e) => {
                      setLinks((v) => {
                        const next = { ...v };
                        if (e.target.checked) next[n.id] = next[n.id] ?? "";
                        else delete next[n.id];
                        return next;
                      });
                    }}
                  />
                  {n.name}
                  <span className="text-micro text-slate-500">{NODE_TYPE_LABELS[n.type] ?? n.type}</span>
                </label>
                {checked && (
                  <input
                    value={links[n.id] ?? ""}
                    onChange={(e) => setLinks((v) => ({ ...v, [n.id]: e.target.value }))}
                    className="ml-auto w-64 bg-black/30 rounded px-2 py-1 text-tiny text-slate-200 outline-none"
                    placeholder={t("certmgr.targetPh")}
                  />
                )}
              </div>
            );
          })}
          {nodes.length === 0 && <span className="text-slate-500">{t("certmgr.noNodesHint")}</span>}
        </div>
      </div>

      {err && <div className="text-caption text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold">
          {initial ? t("certmgr.save") : t("certmgr.createSave")}
        </button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption">{t("certmgr.cancel")}</button>
      </div>
    </div>
  );
}

// ---- 部署节点 ----
function DeployNodes() {
  const { t } = useTranslation();
  const [nodes, setNodes] = useState<DeployNode[]>([]);
  const [editing, setEditing] = useState<DeployNode | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [testMsg, setTestMsg] = useState("");

  const refresh = useCallback(async () => {
    setNodes(await invoke<DeployNode[]>("deploy_node_list"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const del = async (id: string) => {
    if (!confirm(t("certmgr.delNodeConfirm"))) return;
    await invoke("deploy_node_delete", { id });
    refresh();
  };
  const test = async (id: string) => {
    setTestMsg(t("certmgr.testing"));
    try {
      const r = await invoke<string>("deploy_node_test", { id });
      setTestMsg(r);
    } catch (e) {
      setTestMsg(t("certmgr.testFail", { err: String(e) }));
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-bold text-slate-200">{t("certmgr.nodesTitle")}</h3>
        <button onClick={() => { setEditing(null); setShowForm(true); }} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5">
          <Plus className="w-3.5 h-3.5" /> {t("certmgr.newNode")}
        </button>
      </div>

      {showForm && <NodeForm initial={editing} onDone={() => { setShowForm(false); setEditing(null); refresh(); }} />}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {nodes.map((n) => (
          <div key={n.id} className="rounded-ctl border border-white/5 bg-white/5 p-3">
            <div className="flex items-center justify-between">
              <div className="text-body font-semibold text-slate-200">{n.name}</div>
              <span className="text-tiny px-2 py-0.5 rounded bg-sky-500/20 text-sky-300">{NODE_TYPE_LABELS[n.type] ?? n.type}</span>
            </div>
            <div className="text-tiny text-slate-500 mt-1">{t("certmgr.deployInfo", { days: n.deploy_before_days, time: fmtDate(n.last_deploy_at) })}</div>
            {n.last_deploy_error && <div className="text-tiny text-rose-400 mt-1">⚠ {n.last_deploy_error}</div>}
            <div className="flex gap-3 mt-2 text-caption">
              <button onClick={() => { setEditing(n); setShowForm(true); }} className="text-slate-300 hover:text-white">{t("certmgr.edit")}</button>
              <button onClick={() => test(n.id)} className="text-sky-400 hover:text-sky-300 flex items-center gap-1"><Cable className="w-3 h-3" />{t("certmgr.test")}</button>
              <button onClick={() => del(n.id)} className="text-rose-400 hover:text-rose-300 flex items-center gap-1"><Trash2 className="w-3 h-3" />{t("certmgr.delete")}</button>
            </div>
          </div>
        ))}
        {nodes.length === 0 && <div className="text-caption text-slate-500">{t("certmgr.nodesEmpty")}</div>}
      </div>
      {testMsg && <div className="text-caption text-slate-400">{t("certmgr.testLabel", { msg: testMsg })}</div>}
    </div>
  );
}

function NodeForm({ initial, onDone }: { initial: DeployNode | null; onDone: () => void }) {
  const { t } = useTranslation();
  const makeData = (cfg: Record<string, string>, fields: FieldSpec[]) => {
    const d: Record<string, string> = {};
    for (const f of fields) d[f.key] = cfg[f.key] ?? "";
    return d;
  };
  // `type` 是 Rust 侧序列化出来的名字（cert.rs 里 node_type 带 rename = "type"）。
  // 这里不能再写 node_type —— 那样恒为 undefined，任何非 linux 节点都会退化成 linux，
  // 保存时还会把这个错误类型写回去。
  const initialType = initial?.type || "windows";
  const [nodeType, setNodeType] = useState(initialType);
  const [name, setName] = useState(initial?.name || "");
  const [data, setData] = useState<Record<string, string>>(() =>
    makeData(initial?.config ?? {}, NODE_FIELDS[initialType] || [])
  );
  const [deployBefore, setDeployBefore] = useState(initial?.deploy_before_days || 7);
  const [note, setNote] = useState(initial?.note || "");
  const [err, setErr] = useState("");
  // 首次挂载保留 initial 配置；只有用户切换类型时才清空字段
  const firstTypeRun = useRef(true);

  useEffect(() => {
    if (firstTypeRun.current) {
      firstTypeRun.current = false;
      return;
    }
    setData(makeData({}, NODE_FIELDS[nodeType] || []));
  }, [nodeType]);

  const setField = (key: string, val: string) => {
    setData((prev) => ({ ...prev, [key]: val }));
  };

  const submit = async () => {
    setErr("");
    try {
      await invoke("deploy_node_upsert", {
        id: initial?.id,
        nodeType,
        name,
        config: data,
        deployBeforeDays: deployBefore,
        note,
      });
      onDone();
    } catch (e) {
      setErr(String(e));
    }
  };

  const fields = NODE_FIELDS[nodeType] || [];

  return (
    <div className="rounded-ctl border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.name")}
          <input value={name} onChange={(e) => setName(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.type")}
          <select value={nodeType} onChange={(e) => setNodeType(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            {Object.keys(NODE_FIELDS).map((k) => (
              <option key={k} value={k} className="bg-slate-800 text-slate-200">{NODE_TYPE_LABELS[k] ?? k}</option>
            ))}
          </select>
        </label>
      </div>
      <div className="text-caption text-slate-400 font-semibold">{t("certmgr.configItems")}</div>
      <div className="grid grid-cols-2 gap-3">
        {fields.map((f) => (
          <label key={f.key} className={`text-caption text-slate-400 ${f.textarea ? "col-span-2" : ""}`}>
            {t(f.label)}
            {f.select ? (
              <select
                value={data[f.key] ?? ""}
                onChange={(e) => setField(f.key, e.target.value)}
                className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none"
              >
                <option value="">{t("certmgr.select")}</option>
                {f.select.map((o) => (
                  <option key={o.value} value={o.value} className="bg-slate-800 text-slate-200">{o.label}</option>
                ))}
              </select>
            ) : f.textarea ? (
              <textarea
                rows={2}
                value={data[f.key] ?? ""}
                onChange={(e) => setField(f.key, e.target.value)}
                className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none font-mono text-tiny"
                placeholder={f.placeholder || ""}
              />
            ) : (
              <input
                type={f.type}
                value={data[f.key] ?? ""}
                onChange={(e) => setField(f.key, e.target.value)}
                className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none"
                placeholder={f.placeholder || ""}
              />
            )}
          </label>
        ))}
      </div>
      {nodeType === "windows" && (
        <p className="ui-note ui-note-info">
          {t("certmgr.receiverHint")}
        </p>
      )}
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.deployDays")}
          <input type="number" value={deployBefore} onChange={(e) => setDeployBefore(Number(e.target.value))} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.remark")}
          <input value={note} onChange={(e) => setNote(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      {err && <div className="text-caption text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold">{t("certmgr.save")}</button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption">{t("certmgr.cancel")}</button>
      </div>
    </div>
  );
}

// ---- DNS凭据 ----
function Credentials() {
  const { t } = useTranslation();
  const [creds, setCreds] = useState<Credential[]>([]);
  const [showForm, setShowForm] = useState(false);

  const refresh = useCallback(async () => {
    setCreds(await invoke<Credential[]>("credential_list"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const del = async (id: string) => {
    if (!confirm(t("certmgr.delCredConfirm"))) return;
    await invoke("credential_delete", { id });
    refresh();
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-bold text-slate-200">{t("certmgr.credsTitle")}</h3>
        <button onClick={() => setShowForm((v) => !v)} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5">
          <Plus className="w-3.5 h-3.5" /> {t("certmgr.newCred")}
        </button>
      </div>
      {showForm && <CredForm onDone={() => { setShowForm(false); refresh(); }} />}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {creds.map((c) => (
          <div key={c.id} className="rounded-ctl border border-white/5 bg-white/5 p-3">
            <div className="flex items-center justify-between">
              <span className="text-body font-semibold text-slate-200">{c.name}</span>
              <span className="text-tiny px-2 py-0.5 rounded bg-amber-500/20 text-amber-300">{c.type}</span>
            </div>
            <div className="text-tiny text-slate-500 mt-1">{t("certmgr.fieldsLabel", { names: Object.keys(c.data).join(", ") })}</div>
            <div className="mt-2 text-caption">
              <button onClick={() => del(c.id)} className="text-rose-400 hover:text-rose-300 flex items-center gap-1"><Trash2 className="w-3 h-3" />{t("certmgr.delete")}</button>
            </div>
          </div>
        ))}
        {creds.length === 0 && <div className="text-caption text-slate-500">{t("certmgr.credsEmpty")}</div>}
      </div>
    </div>
  );
}

function CredForm({ onDone }: { onDone: () => void }) {
  const { t } = useTranslation();
  const makeData = (fields: FieldSpec[]) => {
    const d: Record<string, string> = {};
    for (const f of fields) d[f.key] = "";
    return d;
  };
  const [name, setName] = useState("");
  const [credType, setCredType] = useState("alidns");
  const [data, setData] = useState<Record<string, string>>(() => makeData(CRED_FIELDS.alidns || []));
  const [customText, setCustomText] = useState("");
  const [note, setNote] = useState("");
  const [err, setErr] = useState("");

  useEffect(() => {
    setData(makeData(CRED_FIELDS[credType] || []));
    setCustomText("");
  }, [credType]);

  const setField = (key: string, val: string) => {
    setData((prev) => ({ ...prev, [key]: val }));
  };

  const isCustom = credType === "custom";

  const submit = async () => {
    setErr("");
    try {
      await invoke("credential_upsert", {
        name,
        credType,
        data: isCustom ? parseKV(customText) : data,
        note,
      });
      onDone();
    } catch (e) {
      setErr(String(e));
    }
  };

  const fields = CRED_FIELDS[credType] || [];

  return (
    <div className="rounded-ctl border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.name")}
          <input value={name} onChange={(e) => setName(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder={t("certmgr.namePh")} />
        </label>
        <label className="text-caption text-slate-400">          {t("certmgr.dnsProvider")}
            <select value={credType} onChange={(e) => setCredType(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            {([...DNS_PROVIDER_MAP, { id: "custom", label: t("certmgr.custom") }]).map((p) => (
              <option key={p.id} value={p.id} className="bg-slate-800 text-slate-200">{t(p.label)}</option>
            ))}
          </select>
        </label>
      </div>
      {isCustom ? (
        <label className="text-caption text-slate-400 block">
          {t("certmgr.envVars")}
          <textarea value={customText} onChange={(e) => setCustomText(e.target.value)} rows={4} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none font-mono text-tiny" placeholder="LEGO_ENV_VAR=value" />
        </label>
      ) : (
        <>
          <div className="text-caption text-slate-400 font-semibold">{t("certmgr.credFields")}</div>
          <div className="grid grid-cols-2 gap-3">
            {fields.map((f) => (
              <label key={f.key} className="text-caption text-slate-400">
                {t(f.label)}
                <input
                  type={f.type}
                  value={data[f.key] ?? ""}
                  onChange={(e) => setField(f.key, e.target.value)}
                  className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none"
                  placeholder={f.placeholder || ""}
                />
              </label>
            ))}
          </div>
        </>
      )}
      <label className="text-caption text-slate-400 block">
        {t("certmgr.remark")}
        <input value={note} onChange={(e) => setNote(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
      </label>
      {err && <div className="text-caption text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold">{t("certmgr.save")}</button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption">{t("certmgr.cancel")}</button>
      </div>
    </div>
  );
}

// ---- 调度日志 ----
function SchedulerView() {
  const { t } = useTranslation();
  const [state, setState] = useState<SchedulerState | null>(null);
  const [running, setRunning] = useState(false);
  const [runResult, setRunResult] = useState<string[]>([]);

  const refresh = useCallback(async () => {
    setState(await invoke<SchedulerState>("cert_scheduler_status"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const toggle = async (enabled: boolean) => {
    try {
      await invoke("cert_scheduler_set", { enabled, intervalMinutes: state?.interval_minutes || 360 });
      await refresh();
    } catch (e) {
      theamedAlert(String(e));
    }
  };
  const runNow = async () => {
    setRunning(true);
    try {
      setRunResult(await invoke<string[]>("cert_scheduler_run_now"));
    } catch (e) {
      setRunResult([String(e)]);
    } finally {
      setRunning(false);
      refresh();
    }
  };

  return (
    <div className="space-y-4">
      <h3 className="text-sm font-bold text-slate-200">{t("certmgr.schedTitle")}</h3>
      <div className="rounded-ctl border border-white/5 bg-white/5 p-4 text-caption text-slate-300 space-y-2">
        <div>{t("certmgr.statusLabel")}{state?.enabled ? <span className="text-emerald-400">{t("certmgr.stateRunning")}</span> : <span className="text-amber-400">{t("certmgr.statePaused")}</span>}</div>
        <div>{t("certmgr.scanInterval", { count: state?.interval_minutes ?? 0 })}</div>
        <div>{t("certmgr.lastRun", { time: fmtDate(state?.last_run_at) })}</div>
        <div>{t("certmgr.nextRun", { time: fmtDate(state?.next_run_at) })}</div>
        <div className="flex gap-2 pt-1">
          <button onClick={() => toggle(!state?.enabled)} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-200 text-caption">
            {state?.enabled ? t("certmgr.pause") : t("certmgr.enable")}
          </button>
          <button onClick={runNow} disabled={running} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption flex items-center gap-1.5">
            <Play className="w-3 h-3" /> {running ? t("certmgr.runningNow") : t("certmgr.runAll")}
          </button>
        </div>
      </div>

      {runResult.length > 0 && (
        <pre className="text-tiny bg-black/40 rounded p-2 overflow-auto max-h-32 text-slate-300">{runResult.join("\n")}</pre>
      )}

      <div>
        <div className="text-caption text-slate-400 mb-2">{t("certmgr.history")}</div>
        <div className="space-y-1">
          {(state?.log || []).slice().reverse().map((e, i) => (
            <div key={i} className="text-tiny flex items-center gap-2">
              <RefreshCw className={`w-3 h-3 ${e.ok ? "text-emerald-400" : "text-rose-400"}`} />
              <span className="text-slate-500">{fmtDate(e.at)}</span>
              <span className="text-slate-300">{e.cert_id}</span>
              <span className="text-slate-400">{e.action}</span>
              <span className={e.ok ? "text-emerald-400" : "text-rose-400"}>{e.message}</span>
            </div>
          ))}
          {(!state?.log || state.log.length === 0) && <div className="text-tiny text-slate-500">{t("certmgr.noRecords")}</div>}
        </div>
      </div>
    </div>
  );
}

// ---- 证书监控（外部域名 / IP） ----
function MonitorView() {
  const { t } = useTranslation();
  const [items, setItems] = useState<MonitorTarget[]>([]);
  const [editing, setEditing] = useState<MonitorTarget | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [checking, setChecking] = useState(false);

  const refresh = useCallback(async () => {
    setItems(await invoke<MonitorTarget[]>("monitor_list"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const checkAll = async () => {
    setChecking(true);
    try {
      setItems(await invoke<MonitorTarget[]>("monitor_check_all"));
    } catch (e) {
      theamedAlert(String(e));
    } finally {
      setChecking(false);
    }
  };
  const checkOne = async (id: string) => {
    try {
      await invoke<MonitorTarget>("monitor_check", { id });
    } catch (e) {
      theamedAlert(String(e));
    }
    refresh();
  };
  const del = async (id: string) => {
    if (!confirm(t("certmgr.delMonitorConfirm"))) return;
    await invoke("monitor_delete", { id });
    refresh();
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-bold text-slate-200">{t("certmgr.monitorsTitle")}</h3>
          <p className="text-tiny text-slate-500 mt-1">{t("certmgr.monitorsHint")}</p>
        </div>
        <div className="flex gap-2">
          <button onClick={checkAll} disabled={checking} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-200 text-caption flex items-center gap-1.5">
            <RefreshCw className={`w-3.5 h-3.5 ${checking ? "animate-spin" : ""}`} /> {checking ? t("certmgr.checking") : t("certmgr.checkAll")}
          </button>
          <button onClick={() => { setEditing(null); setShowForm(true); }} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5">
            <Plus className="w-3.5 h-3.5" /> {t("certmgr.newMonitor")}
          </button>
        </div>
      </div>

      {showForm && (
        <MonitorForm
          initial={editing}
          onDone={() => { setShowForm(false); setEditing(null); refresh(); }}
        />
      )}

      <div className="space-y-2">
        {items.map((m) => {
          const d = m.days_left;
          const tone = d == null ? "text-slate-400" : d <= 7 ? "text-rose-400" : d <= 30 ? "text-amber-400" : "text-emerald-400";
          return (
            <div key={m.id} className="rounded-ctl border border-white/5 bg-white/5 p-3">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="text-body font-semibold text-slate-200 truncate">
                    {m.host}:{m.port}
                    {m.note && <span className="ml-2 text-tiny text-slate-500 font-normal">{m.note}</span>}
                  </div>
                  <div className="text-tiny text-slate-500 mt-1">
                    {t("certmgr.monitorExpire", { time: fmtDate(m.not_after) })} · {t("certmgr.monitorChecked", { time: fmtDate(m.last_check_at) })}
                  </div>
                  {m.subject && <div className="text-tiny text-slate-500 truncate">{m.subject} · {m.issuer}</div>}
                </div>
                <div className="flex items-center gap-3 flex-shrink-0">
                  <span className={`text-caption font-semibold ${tone}`}>
                    {d == null ? "-" : d >= 0 ? t("certmgr.daysLeft", { days: d }) : t("certmgr.daysExpired", { days: -d })}
                  </span>
                  <button onClick={() => checkOne(m.id)} className="text-caption text-sky-400 hover:text-sky-300">{t("certmgr.checkNow")}</button>
                  <button onClick={() => { setEditing(m); setShowForm(true); }} className="text-caption text-slate-300 hover:text-white">{t("certmgr.edit")}</button>
                  <button onClick={() => del(m.id)} className="text-caption text-rose-400 hover:text-rose-300">{t("certmgr.delete")}</button>
                </div>
              </div>
              {m.last_error && <div className="text-tiny text-rose-400 mt-1">⚠ {m.last_error}</div>}
            </div>
          );
        })}
        {items.length === 0 && <div className="text-caption text-slate-500">{t("certmgr.monitorsEmpty")}</div>}
      </div>
    </div>
  );
}

function MonitorForm({ initial, onDone }: { initial: MonitorTarget | null; onDone: () => void }) {
  const { t } = useTranslation();
  const [host, setHost] = useState(initial?.host || "");
  const [port, setPort] = useState(String(initial?.port || 443));
  const [days, setDays] = useState((initial?.notify_days?.length ? initial.notify_days : [30, 14, 7, 3, 1]).join(","));
  const [note, setNote] = useState(initial?.note || "");
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [err, setErr] = useState("");

  const submit = async () => {
    setErr("");
    try {
      const notifyDays = days.split(/[,，\s]+/).map((x) => Number(x.trim())).filter((x) => Number.isFinite(x) && x > 0);
      await invoke("monitor_upsert", {
        id: initial?.id,
        host,
        port: Number(port) || 443,
        note,
        enabled,
        notifyDays,
      });
      onDone();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="rounded-ctl border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-3 gap-3">
        <label className="text-caption text-slate-400 col-span-2">
          {t("certmgr.monitorHost")}
          <input value={host} onChange={(e) => setHost(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="example.com 或 https://example.com:8443" />
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.monitorPort")}
          <input type="number" value={port} onChange={(e) => setPort(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.monitorDays")}
          <input value={days} onChange={(e) => setDays(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="30,14,7,3,1" />
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.remark")}
          <input value={note} onChange={(e) => setNote(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      <label className="flex items-center gap-2 text-caption text-slate-400">
        <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
        {t("certmgr.monitorEnabled")}
      </label>
      {err && <div className="text-caption text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold">{t("certmgr.save")}</button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption">{t("certmgr.cancel")}</button>
      </div>
    </div>
  );
}

// ---- 通知渠道 ----
const NOTIFY_TYPES = [
  { value: "dingtalk", label: "钉钉机器人" },
  { value: "wecom", label: "企业微信机器人" },
  { value: "slack", label: "Slack" },
  { value: "discord", label: "Discord" },
  { value: "teams", label: "Microsoft Teams" },
  { value: "webhook", label: "自定义 Webhook" },
];

function NotifyView() {
  const { t } = useTranslation();
  const [items, setItems] = useState<NotifyChannel[]>([]);
  const [editing, setEditing] = useState<NotifyChannel | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [msg, setMsg] = useState("");

  const refresh = useCallback(async () => {
    setItems(await invoke<NotifyChannel[]>("notify_channel_list"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const test = async (id: string) => {
    setMsg(t("certmgr.testing"));
    try {
      setMsg(await invoke<string>("notify_channel_test", { id }));
    } catch (e) {
      setMsg(t("certmgr.testFail", { err: String(e) }));
    }
  };
  const del = async (id: string) => {
    if (!confirm(t("certmgr.delNotifyConfirm"))) return;
    await invoke("notify_channel_delete", { id });
    refresh();
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-bold text-slate-200">{t("certmgr.notifyTitle")}</h3>
          <p className="text-tiny text-slate-500 mt-1">{t("certmgr.notifyHint")}</p>
        </div>
        <button onClick={() => { setEditing(null); setShowForm(true); }} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5">
          <Plus className="w-3.5 h-3.5" /> {t("certmgr.newNotify")}
        </button>
      </div>

      {showForm && (
        <NotifyForm
          initial={editing}
          onDone={() => { setShowForm(false); setEditing(null); refresh(); }}
        />
      )}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {items.map((c) => (
          <div key={c.id} className="rounded-ctl border border-white/5 bg-white/5 p-3">
            <div className="flex items-center justify-between">
              <div className="text-body font-semibold text-slate-200">{c.name}</div>
              <span className="text-tiny px-2 py-0.5 rounded bg-sky-500/20 text-sky-300">
                {NOTIFY_TYPES.find((x) => x.value === c.type)?.label ?? c.type}
              </span>
            </div>
            <div className="text-tiny text-slate-500 mt-1 break-all">{c.config.url}</div>
            {!c.enabled && <div className="text-tiny text-amber-400 mt-1">{t("certmgr.notifyDisabled")}</div>}
            <div className="flex gap-3 mt-2 text-caption">
              <button onClick={() => { setEditing(c); setShowForm(true); }} className="text-slate-300 hover:text-white">{t("certmgr.edit")}</button>
              <button onClick={() => test(c.id)} className="text-sky-400 hover:text-sky-300">{t("certmgr.sendTest")}</button>
              <button onClick={() => del(c.id)} className="text-rose-400 hover:text-rose-300">{t("certmgr.delete")}</button>
            </div>
          </div>
        ))}
        {items.length === 0 && <div className="text-caption text-slate-500">{t("certmgr.notifyEmpty")}</div>}
      </div>
      {msg && <div className="text-caption text-slate-400">{t("certmgr.testLabel", { msg })}</div>}
    </div>
  );
}

function NotifyForm({ initial, onDone }: { initial: NotifyChannel | null; onDone: () => void }) {
  const { t } = useTranslation();
  const [type, setType] = useState(initial?.type || "dingtalk");
  const [name, setName] = useState(initial?.name || "");
  const [url, setUrl] = useState(initial?.config?.url || "");
  const [secret, setSecret] = useState(initial?.config?.secret || "");
  const [token, setToken] = useState(initial?.config?.token || "");
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [note, setNote] = useState(initial?.note || "");
  const [err, setErr] = useState("");

  const submit = async () => {
    setErr("");
    try {
      const config: Record<string, string> = { url };
      if (type === "dingtalk" && secret) config.secret = secret;
      if (type === "webhook" && token) config.token = token;
      await invoke("notify_channel_upsert", {
        id: initial?.id,
        channelType: type,
        name,
        config,
        enabled,
        note,
      });
      onDone();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="rounded-ctl border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.notifyType")}
          <select value={type} onChange={(e) => setType(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            {NOTIFY_TYPES.map((o) => (
              <option key={o.value} value={o.value} className="bg-slate-800 text-slate-200">{o.label}</option>
            ))}
          </select>
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.name")}
          <input value={name} onChange={(e) => setName(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      <label className="text-caption text-slate-400 block">
        {t("certmgr.notifyUrl")}
        <input value={url} onChange={(e) => setUrl(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="https://..." />
      </label>
      {type === "dingtalk" && (
        <label className="text-caption text-slate-400 block">
          {t("certmgr.notifySecret")}
          <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="SEC..." />
        </label>
      )}
      {type === "webhook" && (
        <label className="text-caption text-slate-400 block">
          {t("certmgr.notifyToken")}
          <input type="password" value={token} onChange={(e) => setToken(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      )}
      <div className="grid grid-cols-2 gap-3">
        <label className="flex items-center gap-2 text-caption text-slate-400">
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
          {t("certmgr.monitorEnabled")}
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.remark")}
          <input value={note} onChange={(e) => setNote(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      {err && <div className="text-caption text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold">{t("certmgr.save")}</button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption">{t("certmgr.cancel")}</button>
      </div>
    </div>
  );
}
