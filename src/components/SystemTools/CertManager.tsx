import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
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
  renew_before_days: number;
  not_before?: string | null;
  not_after?: string | null;
  last_issue_at?: string | null;
  status: string;
  last_error?: string | null;
  deploy_node_ids: string[];
}

interface DeployNode {
  id: string;
  node_type: string;
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
  cred_type: string;
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

type SubTab = "certs" | "nodes" | "creds" | "sched";

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

// ---- 逐字段规格：部署节点配置 ----
type FieldSpec = { key: string; label: string; type: "text" | "password"; placeholder?: string };

const NODE_FIELDS: Record<string, FieldSpec[]> = {
  qiniu: [
    { key: "access_key", label: "AccessKey", type: "text" },
    { key: "secret_key", label: "SecretKey", type: "password" },
    { key: "cdn_domains", label: "CDN 域名（逗号分隔）", type: "text", placeholder: "cdn.example.com" },
  ],
  aliyun: [
    { key: "access_key_id", label: "AccessKey ID", type: "text" },
    { key: "access_key_secret", label: "AccessKey Secret", type: "password" },
    { key: "region", label: "Region", type: "text", placeholder: "cn-hangzhou" },
    { key: "resource_arn", label: "资源 ARN（可选）", type: "text" },
  ],
  linux: [
    { key: "host", label: "主机地址", type: "text", placeholder: "192.168.1.1" },
    { key: "port", label: "SSH 端口", type: "text", placeholder: "22" },
    { key: "user", label: "用户名", type: "text", placeholder: "root" },
    { key: "cert_path", label: "证书路径", type: "text", placeholder: "/etc/nginx/certs/" },
    { key: "key_path", label: "私钥路径", type: "text", placeholder: "/etc/nginx/certs/" },
    { key: "reload_cmd", label: "重载命令", type: "text", placeholder: "nginx -s reload" },
  ],
  windows: [
    { key: "url", label: "接收端地址", type: "text", placeholder: "http://192.168.1.10:9000/push" },
    { key: "token", label: "共享 Token", type: "password" },
  ],
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

// ---------------------------------------------------------------------------
// 组件
// ---------------------------------------------------------------------------
export default function CertManager() {
  const [tab, setTab] = useState<SubTab>(persistedTab);
  const changeTab = (k: SubTab) => {
    persistedTab = k;
    setTab(k);
  };

  return (
    <div className="flex-1 overflow-hidden flex min-h-0 select-none">
      <div className="w-40 flex-shrink-0 border-r border-white/5 py-3 px-2 space-y-0.5 overflow-y-auto">
        {([
          { k: "certs" as const, label: "证书列表", icon: ShieldCheck },
          { k: "nodes" as const, label: "部署节点", icon: Server },
          { k: "creds" as const, label: "DNS凭据", icon: KeyRound },
          { k: "sched" as const, label: "调度日志", icon: History },
        ]).map(({ k, label, icon: Icon }) => (
          <button
            key={k}
            onClick={() => changeTab(k)}
            className={`w-full px-3 py-2 rounded-lg text-[11px] font-semibold flex items-center gap-2 transition-all cursor-pointer text-left ${
              tab === k
                ? "bg-emerald-600 text-white shadow-md shadow-emerald-500/10"
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
        {tab === "sched" && <SchedulerView />}
      </div>
    </div>
  );
}

// ---- 证书列表 ----
function CertList() {
  const [certs, setCerts] = useState<Certificate[]>([]);
  const [creds, setCreds] = useState<Credential[]>([]);
  const [nodes, setNodes] = useState<DeployNode[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [issuing, setIssuing] = useState<Set<string>>(() => new Set(issuingSet));
  const markIssuing = (id: string, on: boolean) => {
    if (on) issuingSet.add(id); else issuingSet.delete(id);
    setIssuing(new Set(issuingSet));
  };
  const [pem, setPem] = useState<Record<string, string> | null>(null);

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
    markIssuing(id, true);
    try {
      await invoke("cert_issue_now", { id });
      await refresh();
    } catch (e) {
      alert("申请失败: " + String(e));
    } finally {
      markIssuing(id, false);
    }
  };
  const del = async (id: string) => {
    if (!confirm("确认删除该证书？")) return;
    await invoke("cert_delete", { id });
    refresh();
  };
  const viewPem = async (id: string) => {
    setPem(await invoke<Record<string, string>>("cert_get_pem", { id }));
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-bold text-slate-200">证书列表</h3>
        <button
          onClick={() => setShowForm((v) => !v)}
          className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-[11px] font-semibold flex items-center gap-1.5"
        >
          <Plus className="w-3.5 h-3.5" /> 新建证书
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

      <div className="overflow-x-auto rounded-lg border border-white/5">
        <table className="w-full text-[11px] text-slate-300">
          <thead className="bg-white/5 text-slate-400">
            <tr>
              <th className="px-3 py-2 text-left">域名</th>
              <th className="px-3 py-2 text-left">CA</th>
              <th className="px-3 py-2 text-left">DNS 插件</th>
              <th className="px-3 py-2 text-left">到期</th>
              <th className="px-3 py-2 text-left">状态</th>
              <th className="px-3 py-2 text-left">部署节点</th>
              <th className="px-3 py-2 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            {certs.map((c) => (
              <tr key={c.id} className="border-t border-white/5">
                <td className="px-3 py-2">{c.domains.join(", ")}</td>
                <td className="px-3 py-2">{c.ca}</td>
                <td className="px-3 py-2">{c.dns_provider}</td>
                <td className="px-3 py-2">{fmtDate(c.not_after)}</td>
                <td className="px-3 py-2">
                  <span className={c.status === "issued" ? "text-emerald-400" : "text-amber-400"}>
                    {c.status}
                  </span>
                </td>
                <td className="px-3 py-2">{c.deploy_node_ids.length} 个</td>
                <td className="px-3 py-2 text-right whitespace-nowrap">
                  <button onClick={() => issue(c.id)} disabled={issuing.has(c.id)} className="text-emerald-400 hover:text-emerald-300 mr-2 disabled:opacity-50">
                    {issuing.has(c.id) ? "申请中…" : "申请"}
                  </button>
                  <button onClick={() => viewPem(c.id)} className="text-sky-400 hover:text-sky-300 mr-2">PEM</button>
                  <button onClick={() => del(c.id)} className="text-rose-400 hover:text-rose-300">
                    <Trash2 className="w-3.5 h-3.5 inline" />
                  </button>
                </td>
              </tr>
            ))}
            {certs.length === 0 && (
              <tr>
                <td colSpan={7} className="px-3 py-6 text-center text-slate-500">暂无证书，点击「新建证书」开始</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {pem && (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onClick={() => setPem(null)}>
          <div className="bg-slate-900 border border-white/10 rounded-xl p-4 w-[80vw] max-h-[80vh] overflow-auto" onClick={(e) => e.stopPropagation()}>
            <div className="flex justify-between mb-2">
              <span className="text-xs font-bold text-slate-200">PEM 文件</span>
              <button onClick={() => setPem(null)} className="text-slate-400 hover:text-slate-200">关闭</button>
            </div>
            {Object.entries(pem).map(([name, content]) => (
              <div key={name} className="mb-3">
                <div className="text-[10px] text-slate-400 mb-1">{name}</div>
                <pre className="text-[9px] bg-black/40 rounded p-2 overflow-auto max-h-40 whitespace-pre-wrap">{content}</pre>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function CertForm({ creds, nodes, onDone }: { creds: Credential[]; nodes: DeployNode[]; onDone: () => void }) {
  const [domains, setDomains] = useState("");
  const [email, setEmail] = useState("");
  const [ca, setCa] = useState("letsencrypt");
  const [dnsProvider, setDnsProvider] = useState("alidns");
  const [credentialId, setCredentialId] = useState("");
  const [renewBefore, setRenewBefore] = useState(30);
  const [deployNodes, setDeployNodes] = useState<string[]>([]);
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
      if (domList.length === 0) return setErr("请填写至少一个域名");
      if (!credentialId) return setErr("请选择关联DNS凭据");
      await invoke("cert_create", {
        domains: domList,
        email,
        ca,
        dnsProvider,
        credentialId,
        renewBeforeDays: renewBefore,
        deployNodeIds: deployNodes,
      });
      onDone();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="rounded-lg border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-[11px] text-slate-400">
          域名（逗号分隔，支持 *.example.com）
          <input value={domains} onChange={(e) => setDomains(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="*.example.com, example.com" />
        </label>
        <label className="text-[11px] text-slate-400">
          邮箱
          <input value={email} onChange={(e) => setEmail(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="admin@example.com" />
        </label>
        <label className="text-[11px] text-slate-400">
          CA
          <select value={ca} onChange={(e) => setCa(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            <option value="letsencrypt" className="bg-slate-800 text-slate-200">letsencrypt（生产）</option>
            <option value="letsencrypt-staging" className="bg-slate-800 text-slate-200">letsencrypt-staging（测试）</option>
          </select>
        </label>
        <label className="text-[11px] text-slate-400">
          DNS 服务商
          <select value={dnsProvider} onChange={(e) => setDnsProvider(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            {[...DNS_PROVIDER_MAP, { id: "custom", label: "custom（自定义）" }].map((p) => (
              <option key={p.id} value={p.id} className="bg-slate-800 text-slate-200">{p.label}</option>
            ))}
          </select>
        </label>
        <label className="text-[11px] text-slate-400">
          关联DNS凭据
          <select value={credentialId} onChange={(e) => setCredentialId(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            <option value="">-- 选择 --</option>
            {creds.map((c) => (
              <option key={c.id} value={c.id}>{c.name}（{c.cred_type}）</option>
            ))}
          </select>
        </label>
        <label className="text-[11px] text-slate-400">
          提前续期天数（N）
          <input type="number" value={renewBefore} onChange={(e) => setRenewBefore(Number(e.target.value))} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      <div className="text-[11px] text-slate-400">
        关联部署节点
        <div className="mt-1 flex flex-wrap gap-2">
          {nodes.map((n) => (
            <label key={n.id} className="flex items-center gap-1 text-[10px] text-slate-300 bg-black/30 rounded px-2 py-1 cursor-pointer">
              <input
                type="checkbox"
                checked={deployNodes.includes(n.id)}
                onChange={(e) => setDeployNodes((v) => (e.target.checked ? [...v, n.id] : v.filter((x) => x !== n.id)))}
              />
              {n.name}
            </label>
          ))}
          {nodes.length === 0 && <span className="text-slate-500">暂无部署节点，请先在「部署节点」中创建</span>}
        </div>
      </div>
      {err && <div className="text-[11px] text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-[11px] font-semibold">创建并保存</button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-lg bg-white/10 text-slate-300 text-[11px]">取消</button>
      </div>
    </div>
  );
}

// ---- 部署节点 ----
function DeployNodes() {
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
    if (!confirm("确认删除该部署节点？关联的证书将自动解除。")) return;
    await invoke("deploy_node_delete", { id });
    refresh();
  };
  const test = async (id: string) => {
    setTestMsg("测试中…");
    try {
      const r = await invoke<string>("deploy_node_test", { id });
      setTestMsg(r);
    } catch (e) {
      setTestMsg("失败: " + String(e));
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-bold text-slate-200">部署节点</h3>
        <button onClick={() => { setEditing(null); setShowForm(true); }} className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-[11px] font-semibold flex items-center gap-1.5">
          <Plus className="w-3.5 h-3.5" /> 新建节点
        </button>
      </div>

      {showForm && <NodeForm initial={editing} onDone={() => { setShowForm(false); setEditing(null); refresh(); }} />}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {nodes.map((n) => (
          <div key={n.id} className="rounded-lg border border-white/5 bg-white/5 p-3">
            <div className="flex items-center justify-between">
              <div className="text-xs font-semibold text-slate-200">{n.name}</div>
              <span className="text-[10px] px-2 py-0.5 rounded bg-sky-500/20 text-sky-300">{n.node_type}</span>
            </div>
            <div className="text-[10px] text-slate-500 mt-1">提前 {n.deploy_before_days} 天部署 · 上次部署 {fmtDate(n.last_deploy_at)}</div>
            {n.last_deploy_error && <div className="text-[10px] text-rose-400 mt-1">⚠ {n.last_deploy_error}</div>}
            <div className="flex gap-3 mt-2 text-[11px]">
              <button onClick={() => { setEditing(n); setShowForm(true); }} className="text-slate-300 hover:text-white">编辑</button>
              <button onClick={() => test(n.id)} className="text-sky-400 hover:text-sky-300 flex items-center gap-1"><Cable className="w-3 h-3" />测试</button>
              <button onClick={() => del(n.id)} className="text-rose-400 hover:text-rose-300 flex items-center gap-1"><Trash2 className="w-3 h-3" />删除</button>
            </div>
          </div>
        ))}
        {nodes.length === 0 && <div className="text-[11px] text-slate-500">暂无部署节点</div>}
      </div>
      {testMsg && <div className="text-[11px] text-slate-400">测试：{testMsg}</div>}
    </div>
  );
}

function NodeForm({ initial, onDone }: { initial: DeployNode | null; onDone: () => void }) {
  const makeData = (cfg: Record<string, string>, fields: FieldSpec[]) => {
    const d: Record<string, string> = {};
    for (const f of fields) d[f.key] = cfg[f.key] ?? "";
    return d;
  };
  const [nodeType, setNodeType] = useState(initial?.node_type || "linux");
  const [name, setName] = useState(initial?.name || "");
  const [data, setData] = useState<Record<string, string>>(() =>
    makeData(initial?.config ?? {}, NODE_FIELDS[initial?.node_type || "linux"] || [])
  );
  const [deployBefore, setDeployBefore] = useState(initial?.deploy_before_days || 7);
  const [note, setNote] = useState(initial?.note || "");
  const [err, setErr] = useState("");

  useEffect(() => {
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
    <div className="rounded-lg border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-[11px] text-slate-400">
          名称
          <input value={name} onChange={(e) => setName(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
        <label className="text-[11px] text-slate-400">
          类型
          <select value={nodeType} onChange={(e) => setNodeType(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            {Object.keys(NODE_FIELDS).map((t) => (
              <option key={t} value={t} className="bg-slate-800 text-slate-200">{t}</option>
            ))}
          </select>
        </label>
      </div>
      <div className="text-[11px] text-slate-400 font-semibold">配置项</div>
      <div className="grid grid-cols-2 gap-3">
        {fields.map((f) => (
          <label key={f.key} className="text-[11px] text-slate-400">
            {f.label}
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
      <div className="grid grid-cols-2 gap-3">
        <label className="text-[11px] text-slate-400">
          提前部署天数（N）
          <input type="number" value={deployBefore} onChange={(e) => setDeployBefore(Number(e.target.value))} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
        <label className="text-[11px] text-slate-400">
          备注
          <input value={note} onChange={(e) => setNote(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      {err && <div className="text-[11px] text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-[11px] font-semibold">保存</button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-lg bg-white/10 text-slate-300 text-[11px]">取消</button>
      </div>
    </div>
  );
}

// ---- DNS凭据 ----
function Credentials() {
  const [creds, setCreds] = useState<Credential[]>([]);
  const [showForm, setShowForm] = useState(false);

  const refresh = useCallback(async () => {
    setCreds(await invoke<Credential[]>("credential_list"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const del = async (id: string) => {
    if (!confirm("确认删除该DNS凭据？")) return;
    await invoke("credential_delete", { id });
    refresh();
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-bold text-slate-200">DNS凭据</h3>
        <button onClick={() => setShowForm((v) => !v)} className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-[11px] font-semibold flex items-center gap-1.5">
          <Plus className="w-3.5 h-3.5" /> 新建DNS凭据
        </button>
      </div>
      {showForm && <CredForm onDone={() => { setShowForm(false); refresh(); }} />}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {creds.map((c) => (
          <div key={c.id} className="rounded-lg border border-white/5 bg-white/5 p-3">
            <div className="flex items-center justify-between">
              <span className="text-xs font-semibold text-slate-200">{c.name}</span>
              <span className="text-[10px] px-2 py-0.5 rounded bg-amber-500/20 text-amber-300">{c.cred_type}</span>
            </div>
            <div className="text-[10px] text-slate-500 mt-1">字段：{Object.keys(c.data).join(", ")}</div>
            <div className="mt-2 text-[11px]">
              <button onClick={() => del(c.id)} className="text-rose-400 hover:text-rose-300 flex items-center gap-1"><Trash2 className="w-3 h-3" />删除</button>
            </div>
          </div>
        ))}
        {creds.length === 0 && <div className="text-[11px] text-slate-500">暂无DNS凭据</div>}
      </div>
    </div>
  );
}

function CredForm({ onDone }: { onDone: () => void }) {
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
    <div className="rounded-lg border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-[11px] text-slate-400">
          名称
          <input value={name} onChange={(e) => setName(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="例如：阿里云生产 DNS" />
        </label>
        <label className="text-[11px] text-slate-400">
          DNS 服务商
          <select value={credType} onChange={(e) => setCredType(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            {([...DNS_PROVIDER_MAP, { id: "custom", label: "custom（自定义）" }]).map((p) => (
              <option key={p.id} value={p.id} className="bg-slate-800 text-slate-200">{p.label}</option>
            ))}
          </select>
        </label>
      </div>
      {isCustom ? (
        <label className="text-[11px] text-slate-400 block">
          lego 环境变量（每行 KEY=VALUE）
          <textarea value={customText} onChange={(e) => setCustomText(e.target.value)} rows={4} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none font-mono text-[10px]" placeholder="LEGO_ENV_VAR=value" />
        </label>
      ) : (
        <>
          <div className="text-[11px] text-slate-400 font-semibold">凭据字段（敏感值存储时做轻量混淆）</div>
          <div className="grid grid-cols-2 gap-3">
            {fields.map((f) => (
              <label key={f.key} className="text-[11px] text-slate-400">
                {f.label}
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
      <label className="text-[11px] text-slate-400 block">
        备注
        <input value={note} onChange={(e) => setNote(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
      </label>
      {err && <div className="text-[11px] text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-[11px] font-semibold">保存</button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-lg bg-white/10 text-slate-300 text-[11px]">取消</button>
      </div>
    </div>
  );
}

// ---- 调度日志 ----
function SchedulerView() {
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
    await invoke("cert_scheduler_set", { enabled, intervalMinutes: state?.interval_minutes || 360 });
    refresh();
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
      <h3 className="text-sm font-bold text-slate-200">调度日志</h3>
      <div className="rounded-lg border border-white/5 bg-white/5 p-4 text-[11px] text-slate-300 space-y-2">
        <div>状态：{state?.enabled ? <span className="text-emerald-400">运行中</span> : <span className="text-amber-400">已暂停</span>}</div>
        <div>扫描间隔：{state?.interval_minutes} 分钟</div>
        <div>上次运行：{fmtDate(state?.last_run_at)}</div>
        <div>下次运行：{fmtDate(state?.next_run_at)}</div>
        <div className="flex gap-2 pt-1">
          <button onClick={() => toggle(!state?.enabled)} className="px-3 py-1.5 rounded-lg bg-white/10 text-slate-200 text-[11px]">
            {state?.enabled ? "暂停" : "启用"}
          </button>
          <button onClick={runNow} disabled={running} className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-[11px] flex items-center gap-1.5">
            <Play className="w-3 h-3" /> {running ? "执行中…" : "立即运行全部"}
          </button>
        </div>
      </div>

      {runResult.length > 0 && (
        <pre className="text-[10px] bg-black/40 rounded p-2 overflow-auto max-h-32 text-slate-300">{runResult.join("\n")}</pre>
      )}

      <div>
        <div className="text-[11px] text-slate-400 mb-2">历史记录</div>
        <div className="space-y-1">
          {(state?.log || []).slice().reverse().map((e, i) => (
            <div key={i} className="text-[10px] flex items-center gap-2">
              <RefreshCw className={`w-3 h-3 ${e.ok ? "text-emerald-400" : "text-rose-400"}`} />
              <span className="text-slate-500">{fmtDate(e.at)}</span>
              <span className="text-slate-300">{e.cert_id}</span>
              <span className="text-slate-400">{e.action}</span>
              <span className={e.ok ? "text-emerald-400" : "text-rose-400"}>{e.message}</span>
            </div>
          ))}
          {(!state?.log || state.log.length === 0) && <div className="text-[10px] text-slate-500">暂无记录</div>}
        </div>
      </div>
    </div>
  );
}
