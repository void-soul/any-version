// Buddy 模块：WorkBuddy / CodeBuddy CN 账号管理（复刻 cockpit-tools）。
// 功能：导入/导出、新增（OAuth / 粘贴 Token）、用量、手动/自动签到、会话管理、切换账号。
import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import {
  RefreshCw,
  Download,
  Upload,
  Trash2,
  Check,
  Loader2,
  Users,
  AlertTriangle,
  CircleCheck,
  Plus,
  KeyRound,
  ExternalLink,
  Gauge,
  MessageSquareText,
  CalendarCheck,
  Play,
  Save,
  Eraser,
  ClipboardCopy,
  FolderOpen,
  Search,
  LogIn,
  X,
  ChevronDown,
  ChevronRight,
  Folder,
  Settings,
  Bell,
  ListChecks,
  Cat,
} from "lucide-react";

export interface BuddyAccount {
  id: string;
  platform: string;
  email: string;
  uid?: string | null;
  nickname?: string | null;
  enterpriseId?: string | null;
  enterpriseName?: string | null;
  tags?: string[] | null;
  accessToken: string;
  refreshToken?: string | null;
  tokenType?: string | null;
  expiresAt?: number | null;
  domain?: string | null;
  planType?: string | null;
  dosageNotifyCode?: string | null;
  dosageNotifyZh?: string | null;
  dosageNotifyEn?: string | null;
  paymentType?: string | null;
  quotaRaw?: unknown;
  usageRaw?: unknown;
  profileRaw?: unknown;
  status?: string | null;
  statusReason?: string | null;
  quotaQueryLastError?: string | null;
  quotaQueryLastErrorAt?: number | null;
  usageUpdatedAt?: number | null;
  lastCheckinTime?: number | null;
  checkinStreak?: number;
  checkinRewards?: unknown;
  authRaw?: unknown;
  expiryTimes?: Record<string, number>;
  createdAt: number;
  lastUsed: number;
}

export interface ExpiryColumn {
  id: string;
  name: string;
}

export interface BuddyPaths {
  platform: string;
  dataDir?: string | null;
  stateDb?: string | null;
  authFile?: string | null;
}

export interface BuddyTransferReport {
  addedConversations: number;
  replacedConversations: number;
  updatedSessionRows: number;
  scannedWorkspaces: number;
}

export interface BuddyClientPath {
  platform: string;
  label: string;
  configured?: string | null;
  resolved?: string | null;
}

export interface BuddySwitchProgress {
  platform: string;
  accountId: string;
  stage: "closing" | "merging" | "writing" | "launching" | "done";
  scannedWorkspaces: number;
  message?: string | null;
}

export interface BuddySessionRecord {
  conversationId: string;
  title: string;
  cwd: string;
  userId: string;
  status: string;
  createdAt?: number | null;
  updatedAt?: number | null;
  isPlayground: boolean;
  locations: { instanceId: string; instanceName: string }[];
}

/** 后端 buddy_delete_sessions 的删除结果 */
export interface BuddySessionDeleteReport {
  dbDeleted: number;
  historyDirsRemoved: number;
  auxiliaryDirsRemoved: number;
  errors: string[];
}

export interface BuddyAutoCheckinConfig {
  enabled: boolean;
  startTime: string;
  endTime: string;
  lastCheckedDate?: string | null;
  accountSchedules?: Record<
    string,
    { scheduledDate: string; scheduledMinute: number; lastCheckedDate?: string | null }
  > | null;
}

/** 单个账号的当日旅行计划与状态 */
export interface BuddyAccountTravelState {
  scheduledDate: string;
  scheduledMinute: number;
  lastDepartDate?: string | null;
  /** 实际派出时刻（HH:MM:SS） */
  lastDepartTime?: string | null;
  lastDoneDate?: string | null;
  lastRewardCredit?: number | null;
}

export interface BuddyAutoTravelConfig {
  enabled: boolean;
  startTime: string;
  endTime: string;
  locationId: number;
  accountSchedules?: Record<string, BuddyAccountTravelState> | null;
}

export interface BuddyActionLogEntry {
  id: string;
  timestamp: string;
  date: string;
  kind: "checkin" | "travel";
  accountId: string;
  email: string;
  status: string;
  message?: string | null;
  credit?: number | null;
}

/** 单个账号的今日签到任务（后端 buddy_auto_checkin_tasks 返回） */
export interface BuddyCheckinTask {
  accountId: string;
  email: string;
  status: "pending" | "success" | "failed";
  scheduledTime?: string | null;
  lastAttemptTime?: string | null;
  /** 实际签到时刻（HH:MM:SS，当日已签到时存在） */
  lastCheckinTime?: string | null;
  message?: string | null;
}

export interface BuddyCheckinTasksView {
  enabled: boolean;
  startTime: string;
  endTime: string;
  /** 今日计划是否已生成（到达开始时间后才生成） */
  generated: boolean;
  tasks: BuddyCheckinTask[];
}

export interface CheckinStatus {
  todayCheckedIn: boolean;
  active: boolean;
  streakDays: number;
  dailyCredit: number;
}

export interface OAuthStartResponse {
  loginId: string;
  verificationUri: string;
  verificationUriComplete?: string | null;
  expiresIn: number;
  intervalSeconds: number;
}

interface QuotaItem {
  packageName: string;
  used: number;
  total: number;
  remain: number;
  unlimited: boolean;
  cycleEndTime?: string | null;
}

const PLATFORMS = [
  { id: "codebuddy-cn", label: "CodeBuddy CN", emoji: "🇨🇳" },
  { id: "workbuddy", label: "WorkBuddy", emoji: "🟣" },
] as const;

type Tab = "accounts" | "sessions" | "checkin" | "settings";

/** 派 Buddy 旅行可选地点（与后端 location_id 对应） */
const TRAVEL_LOCATIONS = [
  { id: 1, key: "loc1" },
  { id: 2, key: "loc2" },
  { id: 3, key: "loc3" },
  { id: 4, key: "loc4" },
] as const;

const localDateStr = () => {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(
    d.getDate()
  ).padStart(2, "0")}`;
};

const fmtMinute = (m: number) =>
  `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;

/** "HH:MM:SS" 加 N 分钟 → "HH:MM" */
const addMinutes = (time: string, minutes: number) => {
  const [h, m] = time.split(":").map(Number);
  if (Number.isNaN(h) || Number.isNaN(m)) return time;
  return fmtMinute((h * 60 + m + minutes) % 1440);
};


// ─── 用量解析 ───

function parseQuotaItems(quotaRaw: unknown): QuotaItem[] {
  const ur = (quotaRaw as Record<string, unknown> | null)?.userResource as
    | Record<string, unknown>
    | undefined;
  const accounts = (ur as Record<string, unknown> | null)?.data as
    | Record<string, unknown>
    | undefined;
  const list = (accounts as Record<string, unknown> | null)?.Response as
    | Record<string, unknown>
    | undefined;
  const arr = ((list as Record<string, unknown> | null)?.Data as
    | Record<string, unknown>
    | null)?.Accounts;
  if (!Array.isArray(arr)) return [];
  return arr
    .map((a) => {
      const r = a as Record<string, unknown>;
      const num = (v: unknown): number => {
        if (typeof v === "number") return v;
        if (typeof v === "string") {
          const n = parseFloat(v);
          return Number.isFinite(n) ? n : 0;
        }
        return 0;
      };
      return {
        packageName: String(r.PackageName ?? r.packageName ?? ""),
        used: num(r.CycleCapacityUsed ?? r.CapacityUsed ?? r.used),
        total: num(r.CycleCapacitySize ?? r.CapacitySize ?? r.total),
        remain: num(r.CycleCapacityRemain ?? r.CapacityRemain ?? r.remain),
        unlimited: Boolean(r.Unlimited),
        cycleEndTime:
          (r.CycleEndTime as string | undefined) ?? (r.cycleEndTime as string | undefined) ?? null,
      };
    })
    .filter((item) => item.total > 0 || item.unlimited || item.packageName);
}

function getDosageText(account: BuddyAccount): string | null {
  const zh = account.dosageNotifyZh?.trim();
  const en = account.dosageNotifyEn?.trim();
  const lang = navigator.language?.toLowerCase() ?? "";
  const text = lang.startsWith("zh") ? zh : en;
  return text || zh || en || null;
}

function getPlanBadge(account: BuddyAccount): string {
  const quota = account.quotaRaw as Record<string, unknown> | null;
  const ur = quota?.userResource as Record<string, unknown> | null;
  const data = (ur as Record<string, unknown> | null)?.data as Record<string, unknown> | null;
  const resp = (data as Record<string, unknown> | null)?.Response as Record<string, unknown> | null;
  const inner = (resp as Record<string, unknown> | null)?.Data as Record<string, unknown> | null;
  const accounts = inner?.Accounts;
  if (Array.isArray(accounts) && accounts.length > 0) {
    const first = accounts[0] as Record<string, unknown>;
    const code = String(first.PackageCode ?? "");
    if (code.includes("enterprise")) return "ENTERPRISE";
    if (code.includes("003") || code.includes("002") || code.includes("005")) return "PRO";
    if (code.includes("039") || code.includes("040")) return "TRIAL";
    return "FREE";
  }
  return "UNKNOWN";
}

function formatQuotaNumber(value: number): string {
  if (!Number.isFinite(value)) return "0";
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(Math.max(0, value));
}

// 总额度（不区分个人体验/裂变包，全部合并为一个 remain/total）
function summarizeQuota(items: QuotaItem[]): {
  used: number;
  total: number;
  remain: number;
  unlimited: boolean;
  hasData: boolean;
} {
  let used = 0;
  let total = 0;
  let remain = 0;
  let unlimited = false;
  for (const it of items) {
    if (it.unlimited) {
      unlimited = true;
      continue;
    }
    used += it.used;
    total += it.total;
    remain += it.remain;
  }
  return { used, total, remain, unlimited, hasData: items.length > 0 };
}

// 时间标签倒计时：距目标还有多久 / 已过期多久
function formatDuration(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const d = Math.floor(totalSec / 86400);
  const h = Math.floor((totalSec % 86400) / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  if (d > 0) return `${d}d${h}h`;
  if (h > 0) return `${h}h${m}m`;
  return `${m}m`;
}

// 恢复进度条的填充窗口：限额窗口多为小时级，取最后 6h 由 0→100%
const RECOVER_HORIZON_MS = 6 * 3600000;

interface LabelState {
  // true = 已过恢复时刻（额度已恢复、可用）；false = 仍在等待恢复（倒计时）
  recovered: boolean;
  progress: number;
  tone: "sky" | "amber" | "emerald";
  text: string;
  absolute: string;
}

function describeLabel(ts: number, now: number): LabelState {
  const absolute = new Date(ts).toLocaleString();
  const diff = ts - now;
  if (diff <= 0) {
    // 时间标签记录的是"恢复时刻"：此刻已过 → 额度已恢复（绿色，进度满）
    return { recovered: true, progress: 100, tone: "emerald", text: formatDuration(-diff), absolute };
  }
  // 尚未到恢复时刻：进度随临近而增长，最后 30 分钟转琥珀提示"即将恢复"
  const progress = Math.max(0, Math.min(100, Math.round((1 - diff / RECOVER_HORIZON_MS) * 100)));
  const tone: LabelState["tone"] = diff < 30 * 60000 ? "amber" : "sky";
  return { recovered: false, progress, tone, text: formatDuration(diff), absolute };
}

function formatTime(t: number | null | undefined): string {
  if (!t) return "—";
  return new Date(t * 1000).toLocaleString();
}

// 宽松解析用户输入/粘贴的时间：自动去空格、统一分隔符，容忍
// "2026-09-10 19:45:01" / "2026/9/10 19:45" / "2026.09.10T19:45" / "2026-09-10"（本地时区）
function parseTimeInput(raw: string): number | null {
  if (!raw) return null;
  let s = raw.trim();
  if (!s) return null;
  s = s
    .replace(/[T]/gi, " ")
    .replace(/[./]/g, "-")
    .replace(/年|月/g, "-")
    .replace(/日/g, " ")
    .replace(/\s{2,}/g, " ")
    .trim();
  const m = s.match(
    /^(\d{4})-(\d{1,2})-(\d{1,2})(?:[ ](\d{1,2}):(\d{1,2})(?::(\d{1,2}))?)?$/
  );
  if (!m) return null;
  const y = +m[1];
  const mo = +m[2];
  const d = +m[3];
  const h = m[4] ? +m[4] : 0;
  const mi = m[5] ? +m[5] : 0;
  const sec = m[6] ? +m[6] : 0;
  if (mo < 1 || mo > 12 || d < 1 || d > 31 || h > 23 || mi > 59 || sec > 59) return null;
  const date = new Date(y, mo - 1, d, h, mi, sec);
  if (Number.isNaN(date.getTime())) return null;
  // 回读校验，防止 2 月 30 日这类被 Date 顺延
  if (
    date.getFullYear() !== y ||
    date.getMonth() !== mo - 1 ||
    date.getDate() !== d ||
    date.getHours() !== h ||
    date.getMinutes() !== mi ||
    date.getSeconds() !== sec
  ) {
    return null;
  }
  return date.getTime();
}

// 把 epoch 毫秒格式化为输入框友好的 "YYYY-MM-DD HH:mm:ss"
function formatTimeInput(ts: number): string {
  const d = new Date(ts);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function formatCwd(cwd: string): string {
  const normalized = cwd.replace(/\\/g, "/").replace(/\/$/, "");
  if (normalized.length <= 50) return normalized;
  const parts = normalized.split("/").filter(Boolean);
  if (parts.length <= 2) return normalized;
  return `/${parts[0]}/.../${parts.slice(-2).join("/")}`;
}

function formatRelative(t: number | null | undefined): string {
  if (!t) return "—";
  const diff = Date.now() / 1000 - t;
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

// 账号行右侧小图标按钮统一样式
const ACC_BTN =
  "px-1.5 py-1 rounded-md bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 cursor-pointer transition disabled:opacity-40";

// 额度余量进度条：宽 = 剩余占比，色 = 剩余多少（≥50 绿 / ≥20 黄 / <20 红）
function QuotaBar({
  remain,
  total,
  className,
  title,
}: {
  remain: number;
  total: number;
  className?: string;
  title?: string;
}) {
  const ratio = total > 0 ? Math.max(0, Math.min(1, remain / total)) : 0;
  const pct = Math.round(ratio * 100);
  const color = ratio >= 0.5 ? "bg-emerald-500" : ratio >= 0.2 ? "bg-amber-500" : "bg-rose-500";
  return (
    <span className={`inline-flex items-center gap-1 ${className ?? ""}`} title={title}>
      <span className="h-1 flex-1 min-w-[20px] bg-white/10 rounded-full overflow-hidden">
        <span className={`block h-full rounded-full ${color}`} style={{ width: `${pct}%` }} />
      </span>
      <span className="text-slate-400 tabular-nums text-[9px]">{formatQuotaNumber(remain)}</span>
    </span>
  );
}

// 账号表格各列固定宽度（px），表头与行按同一宽度对齐
const COL_INFO = 240;
const COL_QUOTA = 120;
const COL_W = 150;
const COL_ACTIONS = 150;

// 时间标签 chip 配色（sky=等待恢复 / amber=即将恢复 / emerald=已恢复可用）
const LABEL_TONE: Record<LabelState["tone"], { chip: string; bar: string; text: string }> = {
  sky: { chip: "bg-sky-500/10 border-sky-500/25", bar: "bg-sky-500/30", text: "text-sky-300" },
  amber: { chip: "bg-amber-500/10 border-amber-500/30", bar: "bg-amber-500/35", text: "text-amber-300" },
  emerald: { chip: "bg-emerald-500/10 border-emerald-500/30", bar: "bg-emerald-500/40", text: "text-emerald-300" },
};

// ─── 会话按项目（工作目录）分组 ───

interface SessionGroup {
  cwd: string;
  sessions: BuddySessionRecord[];
  latestUpdatedAt: number;
}

function buildSessionGroups(sessions: BuddySessionRecord[]): SessionGroup[] {
  const groups = new Map<string, BuddySessionRecord[]>();
  sessions.forEach((s) => {
    const bucket = groups.get(s.cwd) ?? [];
    bucket.push(s);
    groups.set(s.cwd, bucket);
  });
  return Array.from(groups.entries())
    .map(([cwd, groupSessions]) => ({
      cwd,
      sessions: [...groupSessions].sort(
        (a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0) || a.title.localeCompare(b.title)
      ),
      latestUpdatedAt: Math.max(...groupSessions.map((s) => s.updatedAt ?? 0), 0),
    }))
    .sort((a, b) => b.latestUpdatedAt - a.latestUpdatedAt || a.cwd.localeCompare(b.cwd, "zh-CN"));
}

function resolveGroupLabel(cwd: string): string {
  const normalized = cwd.replace(/\\/g, "/").replace(/\/$/, "");
  const parts = normalized.split("/").filter(Boolean);
  return parts[parts.length - 1] || cwd || "（无目录）";
}

export default function BuddyPanel() {
  const { t } = useTranslation();
  const [platform, setPlatform] = useState<string>("codebuddy-cn");
  const [tab, setTab] = useState<Tab>("accounts");
  const [accounts, setAccounts] = useState<BuddyAccount[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [paths, setPaths] = useState<BuddyPaths | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  // 切换进度（关闭 → 合并 → 写入 → 启动），由后端 buddy-switch-progress 事件驱动
  const [switchProgress, setSwitchProgress] = useState<BuddySwitchProgress | null>(null);
  // 客户端路径设置（切换时关闭/重启的 WorkBuddy / CodeBuddy CN）
  const [clientPaths, setClientPaths] = useState<BuddyClientPath[]>([]);
  const [pathDraft, setPathDraft] = useState<Record<string, string>>({});
  const [pathBusy, setPathBusy] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  // 待删除账号：单个删除（卡片按钮）或批量删除（工具栏）共用同一个确认弹窗
  const [deleteIds, setDeleteIds] = useState<string[] | null>(null);
  // 排序模式（额度 / 时间1/2/3 / 默认 last_used 倒序）
  // 过期时间列（CodeBuddy CN 专用）：列 schema + 单元格编辑 + 列增删改 + 用量弹窗 + 倒计时刷新
  const [expiryColumns, setExpiryColumns] = useState<ExpiryColumn[]>([]);
  const [editingCell, setEditingCell] = useState<{ accId: string; colId: string; value: string } | null>(null);
  const [addingColumn, setAddingColumn] = useState(false);
  const [newColumnName, setNewColumnName] = useState("");
  const [editingColumnId, setEditingColumnId] = useState<string | null>(null);
  const [columnNameDraft, setColumnNameDraft] = useState("");
  const [colBusy, setColBusy] = useState(false);
  const [usageAccountId, setUsageAccountId] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  // 新增账号
  const [showAdd, setShowAdd] = useState(false);
  const [addMode, setAddMode] = useState<"oauth" | "token" | "local">("oauth");
  const [oauth, setOauth] = useState<OAuthStartResponse | null>(null);
  const [oauthBusy, setOauthBusy] = useState(false);
  const [tokenInput, setTokenInput] = useState("");

  // 会话
  const [sessions, setSessions] = useState<BuddySessionRecord[]>([]);
  const [sessionKeyword, setSessionKeyword] = useState("");
  const [sessionStatus, setSessionStatus] = useState("");
  const [sessionsBusy, setSessionsBusy] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
  // 待删除的会话（确认弹窗，支持批量）
  const [sessionDelete, setSessionDelete] = useState<{ ids: string[]; label: string } | null>(null);
  // 会话多选（批量删除）
  const [selectedSessionIds, setSelectedSessionIds] = useState<Set<string>>(new Set());

  // 自动签到
  const [autoConfig, setAutoConfig] = useState<BuddyAutoCheckinConfig | null>(null);
  const [actionLogs, setActionLogs] = useState<BuddyActionLogEntry[]>([]);
  const [autoTasks, setAutoTasks] = useState<BuddyCheckinTasksView | null>(null);
  const [travelConfig, setTravelConfig] = useState<BuddyAutoTravelConfig | null>(null);
  const [autoBusy, setAutoBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const [list, current, pathInfo] = await Promise.all([
        invoke<BuddyAccount[]>("buddy_list_accounts", { platform }),
        invoke<string | null>("buddy_get_current_account_id", { platform }),
        invoke<BuddyPaths>("buddy_get_paths", { platform }),
      ]);
      setAccounts(list);
      setCurrentId(current);
      setPaths(pathInfo);
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    }
  }, [platform]);

  useEffect(() => {
    load();
    setSelectedIds(new Set());
  }, [load]);

  // 会话加载（关键字防抖 300ms，与参考实现一致）
  const loadSessions = useCallback(async () => {
    setSessionsBusy(true);
    try {
      const list = await invoke<BuddySessionRecord[]>("buddy_list_sessions", {
        platform,
        keyword: sessionKeyword || null,
        status: sessionStatus || null,
      });
      setSessions(list ?? []);
      // 加载后默认展开全部分组
      setExpandedGroups(new Set((list ?? []).map((s) => s.cwd)));
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    } finally {
      setSessionsBusy(false);
    }
  }, [platform, sessionKeyword, sessionStatus]);

  useEffect(() => {
    if (tab === "sessions") loadSessions();
  }, [tab, loadSessions]);

  // 关键字防抖：输入停止 300ms 后再触发查询
  useEffect(() => {
    const timer = setTimeout(() => {
      if (tab === "sessions") loadSessions();
    }, 300);
    return () => clearTimeout(timer);
  }, [sessionKeyword, tab, loadSessions]);

  // 自动签到配置/行为日志/今日任务列表/旅行配置加载（签到与派出仅限 WorkBuddy）
  const loadAutoCheckin = useCallback(async () => {
    try {
      const [config, logs, tasks, travel] = await Promise.all([
        invoke<BuddyAutoCheckinConfig>("buddy_auto_checkin_get_config"),
        invoke<BuddyActionLogEntry[]>("buddy_get_action_logs"),
        invoke<BuddyCheckinTasksView>("buddy_auto_checkin_tasks"),
        invoke<BuddyAutoTravelConfig>("buddy_auto_travel_get_config"),
      ]);
      setAutoConfig(config);
      setActionLogs(logs ?? []);
      setAutoTasks(tasks);
      setTravelConfig(travel);
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    }
  }, []);

  useEffect(() => {
    if (tab === "checkin") loadAutoCheckin();
  }, [tab, loadAutoCheckin]);

  // 客户端路径设置加载 / 保存 / 清除
  const loadClientPaths = useCallback(async () => {
    try {
      const list = await invoke<BuddyClientPath[]>("buddy_get_client_paths");
      setClientPaths(list);
      setPathDraft({});
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    }
  }, []);

  useEffect(() => {
    if (tab === "settings") void loadClientPaths();
  }, [tab, loadClientPaths]);

  const saveClientPath = async (platform: string, path: string) => {
    setPathBusy(true);
    setMessage(null);
    try {
      await invoke("buddy_set_client_path", { platform, path });
      showMsg(true, path.trim() ? t("buddy.clientPaths.saved") : t("buddy.clientPaths.cleared"));
      await loadClientPaths();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setPathBusy(false);
    }
  };

  const browseClientPath = async (entry: BuddyClientPath) => {
    const selected = await openDialog({
      multiple: false,
      filters: [{ name: "Executable", extensions: ["exe", "app"] }],
      defaultPath: entry.configured ?? entry.resolved ?? undefined,
    });
    if (typeof selected === "string" && selected) {
      setPathDraft((prev) => ({ ...prev, [entry.platform]: selected }));
    }
  };

  // 自动签到事件刷新（日志 / 配置变化都会影响今日任务列表的状态）
  useEffect(() => {
    const unlisteners: (() => void)[] = [];
    const refreshTasks = () => {
      void invoke<BuddyCheckinTasksView>("buddy_auto_checkin_tasks")
        .then(setAutoTasks)
        .catch(() => {});
    };
    const refreshTravel = () => {
      void invoke<BuddyAutoTravelConfig>("buddy_auto_travel_get_config")
        .then(setTravelConfig)
        .catch(() => {});
    };
    const setup = async () => {
      unlisteners.push(
        await listen("buddy-action-logs-changed", () => {
          void invoke<BuddyActionLogEntry[]>("buddy_get_action_logs").then(setActionLogs).catch(() => {});
          refreshTasks();
        })
      );
      unlisteners.push(
        await listen("buddy-auto-checkin-config-changed", () => {
          void invoke<BuddyAutoCheckinConfig>("buddy_auto_checkin_get_config")
            .then(setAutoConfig)
            .catch(() => {});
          refreshTasks();
        })
      );
      unlisteners.push(
        await listen("buddy-auto-travel-config-changed", () => {
          refreshTravel();
        })
      );
    };
    void setup();
    return () => {
      for (const fn of unlisteners) fn();
    };
  }, [platform]);

  const displayName = useMemo(
    () => (acc: BuddyAccount) =>
      acc.nickname?.trim() || acc.email || acc.uid || "unknown",
    []
  );

  const showMsg = (ok: boolean, text: string) => setMessage({ ok, text });

  // 倒计时每 30s 刷新一次（仅账号页需要）
  useEffect(() => {
    if (tab !== "accounts") return;
    const timer = setInterval(() => setNow(Date.now()), 30000);
    return () => clearInterval(timer);
  }, [tab]);

  const usageAccount = useMemo(
    () => accounts.find((a) => a.id === usageAccountId) ?? null,
    [accounts, usageAccountId]
  );

  // 全平台额度聚合（左上角总额度）
  const globalQuota = useMemo(() => {
    let remain = 0;
    let total = 0;
    let unlimited = false;
    let hasData = false;
    for (const a of accounts) {
      const q = summarizeQuota(parseQuotaItems(a.quotaRaw));
      if (!q.hasData) continue;
      hasData = true;
      if (q.unlimited) unlimited = true;
      remain += q.remain;
      total += q.total;
    }
    return { remain, total, unlimited, hasData };
  }, [accounts]);

  // ─── 过期时间列（两平台共享）：列 schema 全局一份，同邮箱账号的时间值互通 ───
  const loadExpiryColumns = useCallback(async () => {
    try {
      const cols = await invoke<ExpiryColumn[]>("buddy_get_expiry_columns");
      setExpiryColumns(cols ?? []);
    } catch (e) {
      showMsg(false, String(e));
    }
  }, []);

  useEffect(() => {
    void loadExpiryColumns();
  }, [loadExpiryColumns]);

  const genColumnId = () =>
    `col_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 7)}`;

  const persistColumns = async (next: ExpiryColumn[]) => {
    setColBusy(true);
    try {
      const saved = await invoke<ExpiryColumn[]>("buddy_set_expiry_columns", { columns: next });
      setExpiryColumns(saved ?? next);
      return saved;
    } catch (e) {
      showMsg(false, String(e));
      return null;
    } finally {
      setColBusy(false);
    }
  };

  const addColumn = async () => {
    const name = newColumnName.trim();
    if (!name) return;
    const saved = await persistColumns([...expiryColumns, { id: genColumnId(), name }]);
    if (saved) {
      setAddingColumn(false);
      setNewColumnName("");
    }
  };

  const startRenameColumn = (col: ExpiryColumn) => {
    setEditingColumnId(col.id);
    setColumnNameDraft(col.name);
  };

  const commitRenameColumn = async (id: string) => {
    const name = columnNameDraft.trim();
    setEditingColumnId(null);
    if (!name) return;
    const cur = expiryColumns.find((c) => c.id === id);
    if (cur && cur.name === name) return;
    await persistColumns(expiryColumns.map((c) => (c.id === id ? { ...c, name } : c)));
  };

  const deleteColumn = async (id: string) => {
    const saved = await persistColumns(expiryColumns.filter((c) => c.id !== id));
    if (saved) {
      setAccounts((prev) =>
        prev.map((a) => {
          if (!a.expiryTimes || !(id in a.expiryTimes)) return a;
          const copy = { ...a.expiryTimes };
          delete copy[id];
          return { ...a, expiryTimes: copy };
        })
      );
      if (editingCell?.colId === id) setEditingCell(null);
    }
  };

  const startEditCell = (acc: BuddyAccount, colId: string) => {
    const ts = acc.expiryTimes?.[colId];
    setEditingCell({ accId: acc.id, colId, value: ts && ts > 0 ? formatTimeInput(ts) : "" });
  };

  const applyCellTime = async (acc: BuddyAccount, colId: string, ts: number | null) => {
    const next = { ...(acc.expiryTimes ?? {}) };
    if (ts == null) delete next[colId];
    else next[colId] = ts;
    try {
      const updated = await invoke<BuddyAccount>("buddy_set_expiry_times", {
        platform,
        accountId: acc.id,
        times: next,
      });
      setAccounts((prev) => prev.map((a) => (a.id === updated.id ? updated : a)));
      setEditingCell(null);
    } catch (e) {
      showMsg(false, String(e));
    }
  };

  const commitCell = (acc: BuddyAccount) => {
    if (!editingCell) return;
    const raw = editingCell.value.trim();
    if (!raw) {
      void applyCellTime(acc, editingCell.colId, null);
      return;
    }
    const parsed = parseTimeInput(raw);
    if (parsed == null) {
      showMsg(false, t("buddy.invalidTimeFormat"));
      return;
    }
    void applyCellTime(acc, editingCell.colId, parsed);
  };

  const clearCell = (acc: BuddyAccount, colId: string) => {
    void applyCellTime(acc, colId, null);
  };

  const normalizedColumnInput = (raw: string) => {
    const p = parseTimeInput(raw);
    return p != null ? formatTimeInput(p) : raw;
  };

  // ─── 账号操作 ───

  const importFromLocal = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const account = await invoke<BuddyAccount | null>("buddy_import_from_local", { platform });
      if (account) {
        showMsg(true, t("buddy.imported", { name: displayName(account) }));
        setShowAdd(false);
      } else {
        showMsg(false, t("buddy.noLocalLogin"));
      }
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setBusy(false);
    }
  };

  // 账号互导：把当前平台全部账号导入到另一平台（目标侧按 uid/email 去重）
  const syncToOther = async () => {
    const other = PLATFORMS.find((p) => p.id !== platform);
    if (!other) return;
    setBusy(true);
    setMessage(null);
    try {
      const count = await invoke<number>("buddy_sync_accounts", {
        fromPlatform: platform,
        toPlatform: other.id,
      });
      showMsg(true, t("buddy.syncImported", { count, target: other.label }));
    } catch (e) {
      showMsg(false, t("buddy.syncFailed", { error: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const switchAccount = async (id: string) => {
    setBusy(true);
    setMessage(null);
    setSwitchProgress(null);
    let unlisten: UnlistenFn | null = null;
    try {
      unlisten = await listen<BuddySwitchProgress>("buddy-switch-progress", (e) =>
        setSwitchProgress(e.payload)
      );
      const [text, report] = await invoke<[string, BuddyTransferReport | null]>(
        "buddy_switch_account",
        { platform, accountId: id }
      );
      const parts = [text];
      if (
        report &&
        report.addedConversations + report.replacedConversations + report.updatedSessionRows + report.scannedWorkspaces >
          0
      ) {
        parts.push(
          t("buddy.mergeResult", {
            added: report.addedConversations,
            replaced: report.replacedConversations,
            rows: report.updatedSessionRows,
            workspaces: report.scannedWorkspaces,
          })
        );
      }
      showMsg(true, parts.join("；"));
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      unlisten?.();
      setSwitchProgress(null);
      setBusy(false);
    }
  };

  const refreshAccount = async (id: string) => {
    setBusy(true);
    setMessage(null);
    try {
      await invoke<BuddyAccount>("buddy_refresh_token", { platform, accountId: id });
      showMsg(true, t("buddy.refreshed"));
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setBusy(false);
    }
  };

  const refreshAll = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const count = await invoke<number>("buddy_refresh_all_tokens", { platform });
      showMsg(true, t("buddy.refreshedAll", { count }));
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setBusy(false);
    }
  };

  const deleteAccounts = async (ids: string[]) => {
    setBusy(true);
    setMessage(null);
    try {
      await invoke("buddy_delete_accounts", { platform, accountIds: ids });
      setSelectedIds((prev) => {
        const next = new Set(prev);
        ids.forEach((id) => next.delete(id));
        return next;
      });
      setDeleteIds(null);
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setBusy(false);
    }
  };

  // 导出指定账号（工具栏批量导出 / 卡片单个导出共用）
  const exportByIds = async (ids: string[], nameBase?: string) => {
    if (ids.length === 0) return;
    try {
      const json = await invoke<string>("buddy_export_accounts", { platform, accountIds: ids });
      const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
      const name = `${nameBase ?? platform}-accounts-${ts}.json`;
      await invoke("file_io_save_text", { fileName: name, content: json });
      showMsg(true, t("buddy.exported", { name }));
    } catch (e) {
      showMsg(false, String(e));
    }
  };

  const exportSelected = async () => exportByIds([...selectedIds]);

  const importFromJson = async () => {
    try {
      const selected = await openDialog({
        title: t("buddy.importJsonPick"),
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!selected) return;
      const path = Array.isArray(selected) ? selected[0] : selected;
      setBusy(true);
      setMessage(null);
      const json = await invoke<string>("read_text_file", { path });
      const imported = await invoke<BuddyAccount[]>("buddy_import_accounts", {
        platform,
        jsonContent: json,
      });
      showMsg(true, t("buddy.importedJson", { count: imported.length }));
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setBusy(false);
    }
  };

  // ─── 新增账号 ───

  const startOAuth = async () => {
    setOauthBusy(true);
    setMessage(null);
    try {
      const res = await invoke<OAuthStartResponse>("buddy_oauth_start", { platform });
      setOauth(res);
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setOauthBusy(false);
    }
  };

  const completeOAuth = async () => {
    if (!oauth) return;
    setOauthBusy(true);
    setMessage(null);
    try {
      const account = await invoke<BuddyAccount>("buddy_oauth_complete", {
        platform,
        loginId: oauth.loginId,
      });
      setShowAdd(false);
      setOauth(null);
      showMsg(true, t("buddy.added", { name: displayName(account) }));
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setOauthBusy(false);
    }
  };

  const cancelOAuth = async () => {
    if (oauth) {
      try {
        await invoke("buddy_oauth_cancel", { platform, loginId: oauth.loginId });
      } catch {
        // 忽略取消失败
      }
    }
    setOauth(null);
    setShowAdd(false);
  };

  const addWithToken = async () => {
    const token = tokenInput.trim();
    if (!token) return;
    setBusy(true);
    setMessage(null);
    try {
      const account = await invoke<BuddyAccount>("buddy_add_account_with_token", {
        platform,
        accessToken: token,
      });
      setShowAdd(false);
      setTokenInput("");
      showMsg(true, t("buddy.added", { name: displayName(account) }));
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setBusy(false);
    }
  };

  // ─── 会话操作 ───

  const copySessionId = async (id: string) => {
    try {
      await navigator.clipboard.writeText(id);
      setCopiedId(id);
      setTimeout(() => setCopiedId(null), 1500);
    } catch {
      // ignore
    }
  };

  const toggleSelect = (id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // ─── 自动签到 ───

  const saveAutoConfig = async () => {
    if (!autoConfig) return;
    setAutoBusy(true);
    setMessage(null);
    try {
      await invoke("buddy_auto_checkin_save_config", { config: autoConfig });
      showMsg(true, t("buddy.autoSaved"));
      await loadAutoCheckin();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setAutoBusy(false);
    }
  };

  const runAutoCheckin = async (force: boolean) => {
    setAutoBusy(true);
    setMessage(null);
    try {
      const result = await invoke<string>("buddy_auto_checkin_run", { force });
      showMsg(true, t(`buddy.autoRun.${result}`, { defaultValue: result }));
      await loadAutoCheckin();
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setAutoBusy(false);
    }
  };

  const clearAutoLogs = async () => {
    try {
      await invoke("buddy_clear_action_logs");
      setActionLogs([]);
    } catch (e) {
      showMsg(false, String(e));
    }
  };

  const saveTravelConfig = async () => {
    if (!travelConfig) return;
    setAutoBusy(true);
    setMessage(null);
    try {
      await invoke("buddy_auto_travel_save_config", { config: travelConfig });
      showMsg(true, t("buddy.travel.saved"));
      await loadAutoCheckin();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setAutoBusy(false);
    }
  };

  const runAutoTravel = async () => {
    setAutoBusy(true);
    setMessage(null);
    try {
      await invoke<string>("buddy_auto_travel_run", { force: true });
      showMsg(true, t("buddy.travel.runDone"));
      await loadAutoCheckin();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setAutoBusy(false);
    }
  };

  // 删除会话（数据库记录 + 本地会话文件，支持批量）
  const deleteSessionsByIds = async (ids: string[]) => {
    if (ids.length === 0) return;
    setSessionsBusy(true);
    try {
      const report = await invoke<BuddySessionDeleteReport>("buddy_delete_sessions", {
        platform,
        conversationIds: ids,
      });
      if (report.errors.length > 0) {
        showMsg(false, `${t("buddy.sessions.deletePartial")}: ${report.errors.join("；")}`);
      } else {
        showMsg(true, t("buddy.sessions.deleteDone"));
      }
      setSelectedSessionIds(new Set());
      await loadSessions();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setSessionsBusy(false);
      setSessionDelete(null);
    }
  };

  const toggleSessionSelect = (id: string) => {
    setSelectedSessionIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleSelectAllSessions = () => {
    setSelectedSessionIds((prev) =>
      sessions.length > 0 && prev.size === sessions.length
        ? new Set()
        : new Set(sessions.map((s) => s.conversationId))
    );
  };

  // 切换平台时清空会话多选（不同平台的会话 id 空间不同）
  useEffect(() => {
    setSelectedSessionIds(new Set());
  }, [platform]);

  const platformLabel = PLATFORMS.find((p) => p.id === platform)?.label ?? platform;
  const otherPlatform = PLATFORMS.find((p) => p.id !== platform);
  const selectedCount = selectedIds.size;

  const tabs: { id: Tab; label: string; icon: React.ReactNode }[] = [
    { id: "accounts", label: t("buddy.tabAccounts"), icon: <Users className="w-3 h-3" /> },
    { id: "sessions", label: t("buddy.tabSessions"), icon: <MessageSquareText className="w-3 h-3" /> },
    { id: "checkin", label: t("buddy.tabCheckin"), icon: <CalendarCheck className="w-3 h-3" /> },
    { id: "settings", label: t("buddy.tabSettings"), icon: <Settings className="w-3 h-3" /> },
  ];

  return (
    <div className="h-full w-full flex flex-col">
      {/* 头部 */}
      <div className="flex items-center gap-2 px-4 py-3 border-b border-white/5 flex-shrink-0">
        <Users className="w-4 h-4 text-[var(--module-accent)]" />
        {/* 平台切换 */}
        <div className="flex items-center gap-1 ml-3 bg-black/30 rounded-lg border border-white/10 p-0.5">
          {PLATFORMS.map((p) => (
            <button
              key={p.id}
              onClick={() => setPlatform(p.id)}
              className={`px-2.5 py-1 rounded-md text-[11px] transition cursor-pointer ${
                platform === p.id
                  ? "bg-[var(--module-accent)]/25 text-white font-semibold"
                  : "text-slate-400 hover:text-white"
              }`}
            >
              <span className="mr-1">{p.emoji}</span>
              {p.label}
            </button>
          ))}
        </div>
        {/* Tab 切换 */}
        <div className="flex items-center gap-1 ml-3 bg-black/30 rounded-lg border border-white/10 p-0.5">
          {tabs.map((tabItem) => (
            <button
              key={tabItem.id}
              onClick={() => setTab(tabItem.id)}
              className={`px-2.5 py-1 rounded-md text-[11px] transition cursor-pointer flex items-center gap-1 ${
                tab === tabItem.id
                  ? "bg-[var(--module-accent)]/25 text-white font-semibold"
                  : "text-slate-400 hover:text-white"
              }`}
            >
              {tabItem.icon}
              {tabItem.label}
            </button>
          ))}
        </div>
        <div className="flex-1" />
        {tab === "accounts" && (
          <>
            <button
              onClick={refreshAll}
              disabled={busy || accounts.length === 0}
              className="px-2.5 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-40"
              title={t("buddy.refreshAllTitle")}
            >
              <RefreshCw className="w-3 h-3" /> {t("buddy.refreshAll")}
            </button>
            <button
              onClick={exportSelected}
              disabled={selectedCount === 0}
              className="px-2.5 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-40"
            >
              <Upload className="w-3 h-3" /> {t("buddy.export")}
            </button>
            <button
              onClick={importFromJson}
              disabled={busy}
              className="px-2.5 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-40"
            >
              <Download className="w-3 h-3" /> {t("buddy.importJson")}
            </button>
            <button
              onClick={() => selectedCount > 0 && setDeleteIds([...selectedIds])}
              disabled={selectedCount === 0}
              className="px-2.5 py-1.5 rounded-lg text-[11px] bg-rose-500/10 hover:bg-rose-500/20 text-rose-300 border border-rose-500/20 flex items-center gap-1 cursor-pointer transition disabled:opacity-40"
            >
              <Trash2 className="w-3 h-3" /> {t("buddy.delete", { count: selectedCount })}
            </button>
            {otherPlatform && (
              <button
                onClick={syncToOther}
                disabled={busy || accounts.length === 0}
                className="px-2.5 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                title={t("buddy.syncImport", { target: otherPlatform.label })}
              >
                <Users className="w-3 h-3" /> {t("buddy.syncImport", { target: otherPlatform.label })}
              </button>
            )}
            <button
              onClick={() => {
                setAddMode("oauth");
                setOauth(null);
                setTokenInput("");
                setShowAdd(true);
              }}
              className="px-2.5 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold flex items-center gap-1 cursor-pointer transition"
            >
              <Plus className="w-3 h-3" /> {t("buddy.addAccount")}
            </button>
          </>
        )}
      </div>

      {/* 切换进度（关闭 → 合并 → 写入 → 启动） */}
      {switchProgress && (
        <div className="flex items-center gap-2 px-4 py-2 text-[11px] border-b bg-sky-500/10 text-sky-300 border-sky-500/20">
          <Loader2 className="w-3 h-3 flex-shrink-0 animate-spin" />
          <span className="break-all">
            {t(`buddy.switchStage.${switchProgress.stage}`, {
              workspaces: switchProgress.scannedWorkspaces,
            })}
          </span>
        </div>
      )}

      {/* 状态提示 */}
      {message && (
        <div
          className={`flex items-center gap-2 px-4 py-2 text-[11px] border-b ${
            message.ok
              ? "bg-emerald-500/10 text-emerald-300 border-emerald-500/20"
              : "bg-rose-500/10 text-rose-300 border-rose-500/20"
          }`}
        >
          {message.ok ? (
            <CircleCheck className="w-3 h-3 flex-shrink-0" />
          ) : (
            <AlertTriangle className="w-3 h-3 flex-shrink-0" />
          )}
          <span className="break-all">{message.text}</span>
          <button onClick={() => setMessage(null)} className="ml-auto text-slate-500 hover:text-white cursor-pointer">✕</button>
        </div>
      )}

      {/* ─── 账号 Tab ─── */}
      {tab === "accounts" && (
        <>
          <div className="flex items-center gap-2 px-4 py-1.5 border-b border-white/5 flex-shrink-0 text-[10px]">
            <span className="text-slate-500">
              {t("buddy.count", { count: accounts.length, platform: platformLabel })}
            </span>
            {globalQuota.hasData && (
              <span className="flex items-center gap-1 text-slate-500">
                <span className="text-slate-600">·</span>
                <span>{t("buddy.quotaTotal")}</span>
                {globalQuota.unlimited ? (
                  <span className="text-emerald-400 tabular-nums">∞</span>
                ) : (
                  <QuotaBar
                    remain={globalQuota.remain}
                    total={globalQuota.total}
                    className="w-20"
                    title={`${formatQuotaNumber(globalQuota.remain)}/${formatQuotaNumber(globalQuota.total)}`}
                  />
                )}
              </span>
            )}
            <div className="flex-1" />
            {addingColumn ? (
              <div className="flex items-center gap-1">
                <input
                  autoFocus
                  value={newColumnName}
                  onChange={(e) => setNewColumnName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void addColumn();
                    } else if (e.key === "Escape") {
                      setAddingColumn(false);
                      setNewColumnName("");
                    }
                  }}
                  placeholder={t("buddy.columnNamePlaceholder")}
                  className="w-28 bg-black/40 border border-white/15 rounded px-1.5 py-1 text-[10px] text-white outline-none placeholder:text-slate-600"
                />
                <button onClick={() => void addColumn()} disabled={colBusy} className="text-emerald-300 disabled:opacity-50 cursor-pointer"><Check className="w-3 h-3" /></button>
                <button onClick={() => { setAddingColumn(false); setNewColumnName(""); }} className="text-slate-500 cursor-pointer"><X className="w-3 h-3" /></button>
              </div>
            ) : (
              <button
                onClick={() => { setAddingColumn(true); setNewColumnName(""); }}
                className="inline-flex items-center gap-0.5 px-1.5 py-1 rounded-md border border-white/10 text-slate-400 hover:text-white hover:border-white/25 cursor-pointer transition"
              >
                <Plus className="w-3 h-3" />{t("buddy.addColumn")}
              </button>
            )}
          </div>

          <div className="flex-1 overflow-auto">
            {accounts.length === 0 ? (
              <div className="h-full flex flex-col items-center justify-center text-slate-500 gap-3 p-6">
                <Users className="w-10 h-10 text-slate-600" />
                <p className="text-xs">{t("buddy.noAccounts")}</p>
                <p className="text-[10px] text-slate-600 max-w-sm text-center">{t("buddy.noAccountsHint")}</p>
                <button
                  onClick={() => {
                    setAddMode("oauth");
                    setOauth(null);
                    setTokenInput("");
                    setShowAdd(true);
                  }}
                  className="mt-1 px-3 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold cursor-pointer transition"
                >
                  {t("buddy.addAccount")}
                </button>
              </div>
            ) : (
              <div style={{ minWidth: COL_INFO + COL_QUOTA + (expiryColumns.length + 1) * COL_W + COL_ACTIONS }}>
                {/* 表头 */}
                <div className="flex items-stretch sticky top-0 z-10 bg-slate-950 border-b border-white/10 text-[10px] text-slate-500">
                  <div style={{ width: COL_INFO }} className="px-3 py-1.5 flex-shrink-0">{t("buddy.colAccount")}</div>
                  <div style={{ width: COL_QUOTA }} className="px-2 py-1.5 flex-shrink-0">{t("buddy.quotaTotal")}</div>
                  {/* 内置「到期」列：登录 token 过期时刻，只读，不属于自定义列 schema */}
                  <div style={{ width: COL_W }} className="px-2 py-1.5 flex-shrink-0 border-l border-white/5">{t("buddy.expiresAt")}</div>
                  {expiryColumns.map((col) => (
                    <div key={col.id} style={{ width: COL_W }} className="px-2 py-1 flex-shrink-0 border-l border-white/5">
                      {editingColumnId === col.id ? (
                        <div className="flex items-center gap-1">
                          <input
                            autoFocus
                            value={columnNameDraft}
                            onChange={(e) => setColumnNameDraft(e.target.value)}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") {
                                e.preventDefault();
                                void commitRenameColumn(col.id);
                              }
                            }}
                            className="min-w-0 flex-1 bg-black/40 border border-white/15 rounded px-1 py-0.5 text-[10px] text-white outline-none"
                          />
                          <button onClick={() => void commitRenameColumn(col.id)} className="text-emerald-300 cursor-pointer"><Check className="w-3 h-3" /></button>
                          <button onClick={() => setEditingColumnId(null)} className="text-slate-500 cursor-pointer"><X className="w-3 h-3" /></button>
                        </div>
                      ) : (
                        <div className="flex items-center gap-1">
                          <button onClick={() => startRenameColumn(col)} title={t("buddy.renameColumn")} className="truncate text-slate-300 hover:text-white cursor-pointer">{col.name}</button>
                          <button onClick={() => void deleteColumn(col.id)} title={t("buddy.deleteColumn")} className="text-slate-600 hover:text-rose-300 cursor-pointer flex-shrink-0"><X className="w-2.5 h-2.5" /></button>
                        </div>
                      )}
                    </div>
                  ))}
                  {/* 操作列表头不显示文字；左分隔线避免顶死时间列 */}
                  <div style={{ width: COL_ACTIONS }} className="px-2 py-1.5 flex-shrink-0 border-l border-white/5" />
                </div>

                {accounts.map((acc) => {
                  const isCurrent = acc.id === currentId;
                  const isSelected = selectedIds.has(acc.id);
                  const quota = summarizeQuota(parseQuotaItems(acc.quotaRaw));
                  const planBadge = getPlanBadge(acc);
                  const cellEditingAcc = editingCell && editingCell.accId === acc.id ? editingCell : null;
                  return (
                    <div
                      key={acc.id}
                      className={`flex items-stretch border-b border-white/5 text-[10px] ${
                        isCurrent ? "bg-emerald-500/[0.06]" : isSelected ? "bg-sky-500/[0.06]" : "hover:bg-white/[0.03]"
                      }`}
                    >
                      {/* 账号信息 */}
                      <div style={{ width: COL_INFO }} className="flex items-center gap-1.5 px-3 py-1.5 flex-shrink-0 min-w-0">
                        <input
                          type="checkbox"
                          checked={isSelected}
                          onChange={() => toggleSelect(acc.id)}
                          className="accent-[var(--module-accent)] w-3 h-3 flex-shrink-0 cursor-pointer"
                        />
                        <span
                          className={`inline-flex items-center text-[9px] px-1.5 py-0.5 rounded border flex-shrink-0 ${
                            planBadge === "PRO"
                              ? "bg-amber-500/15 text-amber-300 border-amber-500/25"
                              : planBadge === "TRIAL"
                                ? "bg-sky-500/15 text-sky-300 border-sky-500/25"
                                : planBadge === "ENTERPRISE"
                                  ? "bg-violet-500/15 text-violet-300 border-violet-500/25"
                                  : "bg-slate-500/15 text-slate-400 border-slate-500/25"
                          }`}
                        >
                          {t(`buddy.plan.${planBadge.toLowerCase()}`, { defaultValue: planBadge })}
                        </span>
                        <span className="font-semibold text-white truncate max-w-[120px]" title={acc.email}>{displayName(acc)}</span>
                        {isCurrent && (
                          <span className="inline-flex items-center gap-0.5 text-[8px] px-1 py-0.5 rounded bg-emerald-500/15 text-emerald-300 border border-emerald-500/25 flex-shrink-0">
                            <Check className="w-2 h-2" /> {t("buddy.current")}
                          </span>
                        )}
                      </div>
                      {/* 额度 */}
                      <div style={{ width: COL_QUOTA }} className="flex items-center px-2 flex-shrink-0">
                        {quota.unlimited ? (
                          <span className="text-emerald-400">∞</span>
                        ) : quota.hasData ? (
                          <QuotaBar
                            remain={quota.remain}
                            total={quota.total}
                            className="w-full"
                            title={`${t("buddy.quotaTotal")}: ${formatQuotaNumber(quota.remain)}/${formatQuotaNumber(quota.total)}`}
                          />
                        ) : (
                          <span className="text-slate-600">—</span>
                        )}
                      </div>
                      {/* 登录 token 到期：只读倒计时（已过期显示「已过期」） */}
                      <div style={{ width: COL_W }} className="px-1.5 py-1 flex-shrink-0 border-l border-white/5">
                        {(() => {
                          const raw = acc.expiresAt ?? 0;
                          if (raw <= 0) return <span className="text-slate-600 px-1">—</span>;
                          const ms = raw < 1e12 ? raw * 1000 : raw; // 秒/毫秒都兼容
                          const st = describeLabel(ms, now);
                          if (st.recovered) {
                            return (
                              <span
                                title={`${t("buddy.expiresAt")} · ${st.absolute}`}
                                className="block w-full rounded border px-1.5 py-0.5 tabular-nums bg-rose-500/10 border-rose-500/25 text-rose-300"
                              >
                                {t("buddy.overdue")}
                              </span>
                            );
                          }
                          const tone = LABEL_TONE[st.tone];
                          return (
                            <span
                              title={`${t("buddy.expiresAt")} · ${st.absolute}`}
                              className={`relative overflow-hidden block rounded border px-1.5 py-0.5 w-full ${tone.chip}`}
                            >
                              <div className={`absolute left-0 top-0 bottom-0 ${tone.bar}`} style={{ width: `${st.progress}%` }} />
                              <span className={`relative tabular-nums ${tone.text}`}>{st.text}</span>
                            </span>
                          );
                        })()}
                      </div>
                      {/* 每个过期时间列一个值 */}
                      {expiryColumns.map((col) => {
                        const ts = acc.expiryTimes?.[col.id];
                        if (cellEditingAcc && cellEditingAcc.colId === col.id) {
                          return (
                            <div key={col.id} style={{ width: COL_W }} className="px-1.5 py-1 flex-shrink-0 border-l border-white/5">
                              <input
                                autoFocus
                                value={cellEditingAcc.value}
                                onChange={(e) => setEditingCell({ ...cellEditingAcc, value: e.target.value })}
                                onBlur={(e) => setEditingCell({ ...cellEditingAcc, value: normalizedColumnInput(e.target.value) })}
                                onKeyDown={(e) => {
                                  if (e.key === "Enter") {
                                    e.preventDefault();
                                    commitCell(acc);
                                  } else if (e.key === "Escape") {
                                    setEditingCell(null);
                                  }
                                }}
                                placeholder={t("buddy.timeInputPlaceholder")}
                                className={`w-full bg-black/40 border rounded px-1 py-0.5 text-[10px] text-white outline-none ${
                                  cellEditingAcc.value.trim() && parseTimeInput(cellEditingAcc.value) == null
                                    ? "border-rose-400/60"
                                    : "border-[var(--module-accent)]/50"
                                }`}
                              />
                              <div className="flex items-center gap-1 mt-1">
                                <button onClick={() => commitCell(acc)} className="px-1.5 py-0.5 rounded bg-[var(--module-accent)] text-white text-[9px] cursor-pointer">{t("buddy.saveLabels")}</button>
                                {ts && ts > 0 && (
                                  <button onClick={() => clearCell(acc, col.id)} className="px-1.5 py-0.5 rounded bg-rose-500/10 text-rose-300 text-[9px] cursor-pointer">{t("buddy.clear")}</button>
                                )}
                                <button onClick={() => setEditingCell(null)} className="px-1.5 py-0.5 rounded text-slate-400 text-[9px] hover:bg-white/5 cursor-pointer">{t("buddy.cancel")}</button>
                              </div>
                            </div>
                          );
                        }
                        if (ts && ts > 0) {
                          const st = describeLabel(ts, now);
                          const tone = LABEL_TONE[st.tone];
                          return (
                            <div key={col.id} style={{ width: COL_W }} className="px-1.5 py-1 flex-shrink-0 border-l border-white/5">
                              <button
                                onClick={() => startEditCell(acc, col.id)}
                                title={`${col.name} · ${st.absolute}`}
                                className={`relative overflow-hidden rounded border px-1.5 py-0.5 w-full text-left ${tone.chip} ${
                                  st.recovered ? "animate-pulse ring-1 ring-emerald-400/70" : ""
                                }`}
                              >
                                <div className={`absolute left-0 top-0 bottom-0 ${tone.bar}`} style={{ width: `${st.progress}%` }} />
                                <span className={`relative tabular-nums flex items-center gap-1 ${tone.text}`}>
                                  {st.recovered ? (<><Bell className="w-2.5 h-2.5" />{t("buddy.recovered")}</>) : st.text}
                                </span>
                              </button>
                            </div>
                          );
                        }
                        return (
                          <div key={col.id} style={{ width: COL_W }} className="px-1.5 py-1 flex-shrink-0 border-l border-white/5">
                            <button
                              onClick={() => startEditCell(acc, col.id)}
                              title={t("buddy.addCellTime")}
                              className="w-full rounded border border-dashed border-white/15 text-slate-600 hover:text-white hover:border-white/30 py-0.5 px-1.5 text-left flex items-center gap-0.5 cursor-pointer"
                            >
                              <Plus className="w-2.5 h-2.5" />
                            </button>
                          </div>
                        );
                      })}
                      {/* 操作 */}
                      <div style={{ width: COL_ACTIONS }} className="flex items-center justify-end gap-1 pl-9 flex-shrink-0 border-l border-white/5">
                        <button onClick={() => switchAccount(acc.id)} disabled={busy || isCurrent} className={ACC_BTN} title={t("buddy.switch")}><LogIn className="w-3 h-3" /></button>
                        <button onClick={() => refreshAccount(acc.id)} disabled={busy} className={ACC_BTN} title={t("buddy.refreshToken")}><RefreshCw className="w-3 h-3" /></button>
                        <button onClick={() => void exportByIds([acc.id])} disabled={busy} className={ACC_BTN} title={t("buddy.exportOne")}><Upload className="w-3 h-3" /></button>
                        <button onClick={() => setDeleteIds([acc.id])} className={`${ACC_BTN} text-rose-300 hover:text-rose-200`} title={t("buddy.deleteOne")}><Trash2 className="w-3 h-3" /></button>
                        <button onClick={() => setUsageAccountId(acc.id)} className={ACC_BTN} title={t("buddy.usage")}><Gauge className="w-3 h-3" /></button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </>
      )}

      {/* ─── 会话 Tab ─── */}
      {tab === "sessions" && (
        <>
          <div className="flex items-center gap-2 px-4 py-2 border-b border-white/5 flex-shrink-0">
            <div className="flex items-center gap-1.5 bg-black/30 rounded-lg border border-white/10 px-2 py-1 flex-1 max-w-xs">
              <Search className="w-3 h-3 text-slate-500" />
              <input
                value={sessionKeyword}
                onChange={(e) => setSessionKeyword(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && loadSessions()}
                placeholder={t("buddy.sessions.searchPlaceholder")}
                className="bg-transparent text-[11px] text-white outline-none w-full placeholder:text-slate-600"
              />
            </div>
            <select
              value={sessionStatus}
              onChange={(e) => setSessionStatus(e.target.value)}
              className="bg-black/30 border border-white/10 rounded-lg px-2 py-1 text-[11px] text-slate-300 outline-none cursor-pointer"
            >
              <option value="">{t("buddy.sessions.statusAll")}</option>
              <option value="Completed">{t("buddy.sessions.statusCompleted")}</option>
              <option value="InProgress">{t("buddy.sessions.statusInProgress")}</option>
            </select>
            <div className="flex-1" />
            <button
              onClick={toggleSelectAllSessions}
              disabled={sessions.length === 0}
              className="px-2 py-1 rounded-md text-[10px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 cursor-pointer transition disabled:opacity-50"
            >
              {sessions.length > 0 && selectedSessionIds.size === sessions.length
                ? t("buddy.sessions.selectAllClear")
                : t("buddy.sessions.selectAll")}
            </button>
            <button
              onClick={() =>
                setSessionDelete({
                  ids: [...selectedSessionIds],
                  label: t("buddy.sessions.deleteBatchLabel", { count: selectedSessionIds.size }),
                })
              }
              disabled={selectedSessionIds.size === 0 || sessionsBusy}
              className="px-2 py-1 rounded-md text-[10px] bg-rose-500/10 hover:bg-rose-500/20 text-rose-300 border border-rose-500/20 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
            >
              <Trash2 className="w-3 h-3" />
              {t("buddy.sessions.deleteSelected", { count: selectedSessionIds.size })}
            </button>
            <button
              onClick={loadSessions}
              disabled={sessionsBusy}
              className="px-2 py-1 rounded-md text-[10px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-40"
            >
              {sessionsBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <RefreshCw className="w-3 h-3" />}
              {t("buddy.refresh")}
            </button>
          </div>
          <div className="flex-1 overflow-y-auto p-4">
            {sessions.length === 0 ? (
              <div className="h-full flex flex-col items-center justify-center text-slate-500 gap-2">
                <MessageSquareText className="w-10 h-10 text-slate-600" />
                <p className="text-xs">{t("buddy.sessions.noSessions")}</p>
              </div>
            ) : (
              <div className="space-y-2">
                {buildSessionGroups(sessions).map((group) => {
                  const isExpanded = expandedGroups.has(group.cwd);
                  return (
                    <div
                      key={group.cwd || "__empty__"}
                      className="rounded-xl border border-white/10 bg-white/[0.03] overflow-hidden"
                    >
                      {/* 项目分组头 */}
                      <div
                        className="flex items-center gap-2 px-3 py-2 hover:bg-white/[0.04] cursor-pointer transition select-none"
                        onClick={() =>
                          setExpandedGroups((prev) => {
                            const next = new Set(prev);
                            if (next.has(group.cwd)) next.delete(group.cwd);
                            else next.add(group.cwd);
                            return next;
                          })
                        }
                      >
                        {isExpanded ? (
                          <ChevronDown className="w-3.5 h-3.5 text-slate-500 flex-shrink-0" />
                        ) : (
                          <ChevronRight className="w-3.5 h-3.5 text-slate-500 flex-shrink-0" />
                        )}
                        <Folder className="w-3.5 h-3.5 text-[var(--module-accent)] flex-shrink-0" />
                        <span className="text-[12px] font-semibold text-white truncate">
                          {resolveGroupLabel(group.cwd)}
                        </span>
                        <span className="inline-flex items-center text-[9px] px-1.5 py-0.5 rounded bg-white/5 text-slate-400 border border-white/10 flex-shrink-0">
                          {group.sessions.length}
                        </span>
                        {group.cwd && (
                          <span className="text-[9px] text-slate-600 truncate hidden sm:inline">
                            {formatCwd(group.cwd)}
                          </span>
                        )}
                        <span className="ml-auto text-[9px] text-slate-500 flex-shrink-0">
                          {formatRelative(group.latestUpdatedAt)}
                        </span>
                      </div>
                      {/* 分组内会话 */}
                      {isExpanded && (
                        <div className="border-t border-white/5 divide-y divide-white/5">
                          {group.sessions.map((s) => (
                            <div
                              key={s.conversationId}
                              className="px-3 py-2.5 hover:bg-white/[0.03] transition flex items-start gap-3"
                            >
                              <input
                                type="checkbox"
                                checked={selectedSessionIds.has(s.conversationId)}
                                onChange={() => toggleSessionSelect(s.conversationId)}
                                className="accent-[var(--module-accent)] w-3 h-3 cursor-pointer flex-shrink-0 mt-0.5"
                                title={t("buddy.sessions.selectOne")}
                              />
                              <div className="flex-1 min-w-0">
                                <div className="flex items-center gap-2">
                                  <span className="text-[11px] font-semibold text-white truncate">
                                    {s.title || "（无标题）"}
                                  </span>
                                  <span
                                    className={`inline-flex items-center text-[8px] px-1 py-0.5 rounded border flex-shrink-0 ${
                                      s.status === "Completed"
                                        ? "bg-emerald-500/15 text-emerald-300 border-emerald-500/25"
                                        : s.status === "InProgress"
                                          ? "bg-sky-500/15 text-sky-300 border-sky-500/25"
                                          : "bg-slate-500/15 text-slate-400 border-slate-500/25"
                                    }`}
                                  >
                                    {s.status}
                                  </span>
                                  {s.isPlayground && (
                                    <span className="inline-flex items-center text-[8px] px-1 py-0.5 rounded bg-violet-500/15 text-violet-300 border border-violet-500/25 flex-shrink-0">
                                      Playground
                                    </span>
                                  )}
                                </div>
                                <div className="text-[10px] text-slate-500 mt-0.5 truncate" title={s.cwd}>
                                  <FolderOpen className="w-2.5 h-2.5 inline mr-1" />
                                  {formatCwd(s.cwd) || "—"}
                                </div>
                                <div className="text-[9px] text-slate-600 mt-0.5 flex items-center gap-2 flex-wrap">
                                  <span>ID: {s.conversationId.slice(0, 12)}…</span>
                                  <span>{t("buddy.sessions.updated")}: {formatRelative(s.updatedAt)}</span>
                                  {s.locations.length > 0 && (
                                    <span>{s.locations.map((l) => l.instanceName).join(", ")}</span>
                                  )}
                                </div>
                              </div>
                              <button
                                onClick={() => void copySessionId(s.conversationId)}
                                className="p-1.5 rounded-md bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 cursor-pointer transition flex-shrink-0"
                                title={t("buddy.sessions.copyId")}
                              >
                                {copiedId === s.conversationId ? (
                                  <Check className="w-3 h-3 text-emerald-400" />
                                ) : (
                                  <ClipboardCopy className="w-3 h-3" />
                                )}
                              </button>
                              <button
                                onClick={() => setSessionDelete({ ids: [s.conversationId], label: s.title || s.conversationId })}
                                className="p-1.5 rounded-md bg-white/5 hover:bg-rose-500/15 text-rose-300 border border-white/10 cursor-pointer transition flex-shrink-0"
                                title={t("buddy.sessions.deleteTitle")}
                              >
                                <Trash2 className="w-3 h-3" />
                              </button>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </>
      )}

      {/* ─── 签到 Tab ─── */}
      {tab === "checkin" && (
        <div className="flex-1 overflow-y-auto p-4">
          <div className="grid grid-cols-1 xl:grid-cols-2 gap-4 items-start">
            {/* 自动签到 + 派旅行配置（合并卡片） */}
            <div className="rounded-xl border border-white/10 bg-white/[0.03] p-4 space-y-4">
              <div>
              <div className="flex items-center gap-2 mb-3">
                <CalendarCheck className="w-4 h-4 text-[var(--module-accent)]" />
                <span className="text-[13px] font-bold text-white">{t("buddy.auto.title")}</span>
              </div>
              {autoConfig ? (
                <div className="space-y-3">
                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={autoConfig.enabled}
                      onChange={(e) => setAutoConfig({ ...autoConfig, enabled: e.target.checked })}
                      className="accent-[var(--module-accent)] w-3.5 h-3.5"
                    />
                    <span className="text-[11px] text-slate-300">{t("buddy.auto.enabled")}</span>
                  </label>
                  <div className="flex items-center gap-3">
                    <label className="flex items-center gap-2">
                      <span className="text-[10px] text-slate-500">{t("buddy.auto.startTime")}</span>
                      <input
                        type="time"
                        value={autoConfig.startTime}
                        onChange={(e) => setAutoConfig({ ...autoConfig, startTime: e.target.value })}
                        className="bg-black/30 border border-white/10 rounded-lg px-2 py-1 text-[11px] text-white outline-none"
                      />
                    </label>
                    <label className="flex items-center gap-2">
                      <span className="text-[10px] text-slate-500">{t("buddy.auto.endTime")}</span>
                      <input
                        type="time"
                        value={autoConfig.endTime}
                        onChange={(e) => setAutoConfig({ ...autoConfig, endTime: e.target.value })}
                        className="bg-black/30 border border-white/10 rounded-lg px-2 py-1 text-[11px] text-white outline-none"
                      />
                    </label>
                  </div>
                  <p className="text-[9px] text-slate-600">{t("buddy.auto.hint")}</p>
                  <div className="flex gap-2">
                    <button
                      onClick={saveAutoConfig}
                      disabled={autoBusy}
                      className="px-3 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                    >
                      {autoBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />}
                      {t("buddy.auto.save")}
                    </button>
                    <button
                      onClick={() => void runAutoCheckin(true)}
                      disabled={autoBusy}
                      className="px-3 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                    >
                      <Play className="w-3 h-3" /> {t("buddy.auto.runNow")}
                    </button>
                  </div>
                </div>
              ) : (
                <div className="text-[10px] text-slate-600">{t("buddy.auto.loading")}</div>
              )}
              </div>

              <div className="border-t border-white/5 pt-4">
                <div className="flex items-center gap-2 mb-3">
                  <Cat className="w-4 h-4 text-[var(--module-accent)]" />
                  <span className="text-[13px] font-bold text-white">{t("buddy.travel.title")}</span>
                </div>
                {travelConfig ? (
                  <div className="space-y-3">
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={travelConfig.enabled}
                        onChange={(e) => setTravelConfig({ ...travelConfig, enabled: e.target.checked })}
                        className="accent-[var(--module-accent)] w-3.5 h-3.5"
                      />
                      <span className="text-[11px] text-slate-300">{t("buddy.travel.enabled")}</span>
                    </label>
                    <div className="flex items-center gap-3 flex-wrap">
                      <label className="flex items-center gap-2">
                        <span className="text-[10px] text-slate-500">{t("buddy.travel.startTime")}</span>
                        <input
                          type="time"
                          value={travelConfig.startTime}
                          onChange={(e) => setTravelConfig({ ...travelConfig, startTime: e.target.value })}
                          className="bg-black/30 border border-white/10 rounded-lg px-2 py-1 text-[11px] text-white outline-none"
                        />
                      </label>
                      <label className="flex items-center gap-2">
                        <span className="text-[10px] text-slate-500">{t("buddy.travel.endTime")}</span>
                        <input
                          type="time"
                          value={travelConfig.endTime}
                          onChange={(e) => setTravelConfig({ ...travelConfig, endTime: e.target.value })}
                          className="bg-black/30 border border-white/10 rounded-lg px-2 py-1 text-[11px] text-white outline-none"
                        />
                      </label>
                      <label className="flex items-center gap-2">
                        <span className="text-[10px] text-slate-500">{t("buddy.travel.location")}</span>
                        <select
                          value={travelConfig.locationId}
                          onChange={(e) =>
                            setTravelConfig({ ...travelConfig, locationId: Number(e.target.value) })
                          }
                          className="bg-black/30 border border-white/10 rounded-lg px-2 py-1 text-[11px] text-white outline-none"
                        >
                          {TRAVEL_LOCATIONS.map((loc) => (
                            <option key={loc.id} value={loc.id}>
                              {t(`buddy.travel.${loc.key}`)}
                            </option>
                          ))}
                        </select>
                      </label>
                    </div>
                    <p className="text-[9px] text-slate-600">{t("buddy.travel.hint")}</p>
                    <div className="flex gap-2">
                      <button
                        onClick={saveTravelConfig}
                        disabled={autoBusy}
                        className="px-3 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                      >
                        {autoBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />}
                        {t("buddy.auto.save")}
                      </button>
                      <button
                        onClick={() => void runAutoTravel()}
                        disabled={autoBusy}
                        className="px-3 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                      >
                        <Play className="w-3 h-3" /> {t("buddy.travel.runNow")}
                      </button>
                    </div>
                  </div>
                ) : (
                  <div className="text-[10px] text-slate-600">{t("buddy.auto.loading")}</div>
                )}
              </div>
            </div>

            {/* 每账号状态（签到 | 派出） */}
            <div className="rounded-xl border border-white/10 bg-white/[0.03] p-4">
              <div className="flex items-center gap-2 mb-3">
                <ListChecks className="w-4 h-4 text-[var(--module-accent)]" />
                <span className="text-[13px] font-bold text-white">{t("buddy.accountStatus.title")}</span>
                <div className="flex-1" />
                <button
                  onClick={() => void loadAutoCheckin()}
                  className="px-2 py-1 rounded-md text-[10px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition"
                >
                  <RefreshCw className="w-3 h-3" /> {t("buddy.auto.refresh")}
                </button>
              </div>
              {accounts.length === 0 ? (
                <div className="text-[10px] text-slate-600">{t("buddy.auto.noTasks")}</div>
              ) : (
                <div className="space-y-2">
                  {accounts.map((acc) => {
                    const task = autoTasks?.tasks.find((x) => x.accountId === acc.id);
                    const sch = travelConfig?.accountSchedules?.[acc.id];
                    const today = localDateStr();
                    // 签到状态
                    let checkinState: string;
                    let checkinCls: string;
                    if (task?.status === "success") {
                      checkinState = t("buddy.accountStatus.checkinDone");
                      checkinCls = "text-emerald-300";
                    } else if (task?.status === "failed") {
                      checkinState = t("buddy.accountStatus.checkinFailed");
                      checkinCls = "text-rose-300";
                    } else if (autoTasks?.generated && task?.scheduledTime) {
                      checkinState = t("buddy.accountStatus.checkinPending");
                      checkinCls = "text-slate-300";
                    } else {
                      checkinState = t("buddy.accountStatus.checkinNotGenerated");
                      checkinCls = "text-slate-400";
                    }
                    // 派出状态
                    const departedToday = sch?.lastDepartDate === today;
                    const traveling = departedToday && sch?.lastDoneDate !== today;
                    let travelState: string;
                    let travelCls: string;
                    if (sch && sch.scheduledDate === today) {
                      if (sch.lastDoneDate === today) {
                        if (sch.lastRewardCredit != null) {
                          travelState = t("buddy.travel.stateClaimed", { credit: sch.lastRewardCredit });
                          travelCls = "text-emerald-300";
                        } else {
                          travelState = t("buddy.travel.stateLimit");
                          travelCls = "text-slate-400";
                        }
                      } else if (departedToday) {
                        travelState = t("buddy.travel.stateTraveling");
                        travelCls = "text-sky-300";
                      } else {
                        travelState = t("buddy.accountStatus.travelPending");
                        travelCls = "text-slate-300";
                      }
                    } else {
                      travelState = t("buddy.travel.stateNotToday");
                      travelCls = "text-slate-500";
                    }
                    return (
                      <div key={acc.id} className="rounded-lg border border-white/5 bg-black/20 px-3 py-2">
                        <div className="text-[11px] text-slate-200 truncate mb-1.5" title={acc.email}>
                          {acc.email || acc.id}
                        </div>
                        <div className="grid grid-cols-2 gap-2">
                          {/* 签到 */}
                          <div className="space-y-0.5 min-w-0">
                            <div className="flex items-center gap-1.5">
                              <CalendarCheck className="w-3 h-3 text-slate-500 flex-shrink-0" />
                              <span className={`text-[10px] font-semibold ${checkinCls}`}>{checkinState}</span>
                            </div>
                            <div className="text-[9px] text-slate-500 pl-[18px]">
                              {t("buddy.accountStatus.planTime", { time: task?.scheduledTime ?? "—" })}
                              {task?.lastCheckinTime
                                ? ` · ${t("buddy.accountStatus.actualTime", { time: task.lastCheckinTime })}`
                                : ""}
                            </div>
                          </div>
                          {/* 派出 */}
                          <div className="space-y-0.5 min-w-0">
                            <div className="flex items-center gap-1.5">
                              <Cat className="w-3 h-3 text-slate-500 flex-shrink-0" />
                              <span className={`text-[10px] font-semibold ${travelCls}`}>{travelState}</span>
                            </div>
                            <div className="text-[9px] text-slate-500 pl-[18px]">
                              {t("buddy.accountStatus.travelCount", {
                                count: departedToday ? 1 : 0,
                              })}
                              {sch?.lastDepartTime
                                ? ` · ${t("buddy.accountStatus.actualTime", { time: sch.lastDepartTime })}`
                                : ""}
                            </div>
                            {traveling && sch?.lastDepartTime && (
                              <div className="text-[9px] text-slate-500 pl-[18px]">
                                {t("buddy.accountStatus.expectedReturn", {
                                  from: addMinutes(sch.lastDepartTime, 60),
                                  to: addMinutes(sch.lastDepartTime, 240),
                                })}
                              </div>
                            )}
                          </div>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>

            {/* 行为日志（签到 + 派旅行，平铺） */}
            <div className="xl:col-span-2 rounded-xl border border-white/10 bg-white/[0.03] p-4">
              <div className="flex items-center gap-2 mb-3">
                <span className="text-[13px] font-bold text-white">{t("buddy.actionLogs.title")}</span>
                <div className="flex-1" />
                <button
                  onClick={clearAutoLogs}
                  className="px-2 py-1 rounded-md text-[10px] bg-rose-500/10 hover:bg-rose-500/20 text-rose-300 border border-rose-500/20 flex items-center gap-1 cursor-pointer transition"
                >
                  <Eraser className="w-3 h-3" /> {t("buddy.auto.clearLogs")}
                </button>
              </div>
              {actionLogs.length === 0 ? (
                <div className="text-[10px] text-slate-600">{t("buddy.actionLogs.empty")}</div>
              ) : (
                <div className="space-y-1 max-h-[40vh] overflow-y-auto">
                  {actionLogs.map((entry) => {
                    const okStatus = ["success", "already_checked", "claimed", "departed"].includes(entry.status);
                    const badStatus = ["failed", "inactive"].includes(entry.status);
                    return (
                      <div key={entry.id} className="flex items-center gap-2 text-[10px] font-mono">
                        <span
                          className={`px-1 py-0.5 rounded border flex-shrink-0 ${
                            entry.kind === "travel"
                              ? "text-sky-300 bg-sky-500/10 border-sky-500/20"
                              : "text-emerald-300 bg-emerald-500/10 border-emerald-500/20"
                          }`}
                        >
                          {entry.kind === "travel" ? t("buddy.actionLogs.kindTravel") : t("buddy.actionLogs.kindCheckin")}
                        </span>
                        <span className="text-slate-500 flex-shrink-0">{entry.timestamp}</span>
                        <span className="text-slate-300 truncate max-w-[160px]" title={entry.email}>
                          {entry.email || entry.accountId}
                        </span>
                        <span
                          className={`truncate flex-1 ${
                            okStatus ? "text-emerald-400" : badStatus ? "text-rose-400" : "text-slate-400"
                          }`}
                        >
                          {entry.status === "claimed" && entry.credit != null
                            ? `${entry.message ?? ""} +${entry.credit}`
                            : entry.message ?? entry.status}
                        </span>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* 路径信息 */}
      {/* ─── 设置 Tab：客户端路径（切换时仅请求关闭，由用户手动启动） ─── */}
      {tab === "settings" && (
        <div className="flex-1 overflow-y-auto px-4 py-3 space-y-3">
          <div className="rounded-xl border border-white/10 bg-white/[0.03] p-4">
            <div className="text-[13px] font-bold text-white mb-1">{t("buddy.clientPaths.title")}</div>
            <p className="text-[9px] text-slate-600 mb-3">{t("buddy.clientPaths.hint")}</p>
            <div className="space-y-3">
              {clientPaths.map((entry) => {
                const draft = pathDraft[entry.platform];
                const current = draft ?? entry.configured ?? "";
                const dirty = current !== (entry.configured ?? "");
                return (
                  <div key={entry.platform} className="rounded-lg border border-white/10 bg-black/20 p-2.5">
                    <div className="flex items-center gap-2 mb-1.5">
                      <span className="text-[11px] font-semibold text-white">{entry.label}</span>
                      <span
                        className={`px-1.5 py-0.5 rounded text-[9px] border ${
                          entry.configured
                            ? "text-sky-300 bg-sky-500/10 border-sky-500/25"
                            : entry.resolved
                              ? "text-emerald-300 bg-emerald-500/10 border-emerald-500/25"
                              : "text-slate-500 bg-white/5 border-white/10"
                        }`}
                      >
                        {entry.configured
                          ? t("buddy.clientPaths.custom")
                          : entry.resolved
                            ? t("buddy.clientPaths.auto")
                            : t("buddy.clientPaths.missing")}
                      </span>
                    </div>
                    <input
                      value={current}
                      onChange={(e) =>
                        setPathDraft((prev) => ({ ...prev, [entry.platform]: e.target.value }))
                      }
                      placeholder={entry.resolved ?? t("buddy.clientPaths.placeholder")}
                      spellCheck={false}
                      className="w-full bg-black/30 border border-white/10 rounded-lg px-2 py-1.5 text-[11px] text-white outline-none focus:border-[var(--module-accent)]/50"
                    />
                    {entry.resolved && (
                      <div className="text-[9px] text-slate-600 mt-1 break-all">
                        {t("buddy.clientPaths.resolved")}: {entry.resolved}
                      </div>
                    )}
                    <div className="flex gap-2 mt-2">
                      <button
                        onClick={() => void browseClientPath(entry)}
                        className="px-2.5 py-1 rounded-md text-[10px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition"
                      >
                        <FolderOpen className="w-3 h-3" /> {t("buddy.clientPaths.browse")}
                      </button>
                      <button
                        onClick={() => void saveClientPath(entry.platform, current)}
                        disabled={pathBusy || !dirty}
                        className="px-2.5 py-1 rounded-md text-[10px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                      >
                        {pathBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />}{" "}
                        {t("buddy.clientPaths.save")}
                      </button>
                      {entry.configured && (
                        <button
                          onClick={() => void saveClientPath(entry.platform, "")}
                          disabled={pathBusy}
                          className="px-2.5 py-1 rounded-md text-[10px] bg-rose-500/10 hover:bg-rose-500/20 text-rose-300 border border-rose-500/20 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                        >
                          <Eraser className="w-3 h-3" /> {t("buddy.clientPaths.clear")}
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      )}

      {paths && (paths.stateDb || paths.authFile || paths.dataDir) && (
        <div className="px-4 py-2 border-t border-white/5 flex-shrink-0 flex flex-wrap gap-x-4 gap-y-1 text-[9px] text-slate-600">
          {paths.dataDir && <span>{t("buddy.dataDir")}: {paths.dataDir}</span>}
          {paths.stateDb && <span>state.vscdb: {paths.stateDb}</span>}
          {paths.authFile && <span>auth: {paths.authFile}</span>}
        </div>
      )}

      {/* 用量详情弹窗（各账号「用量」按钮） */}
      {usageAccount &&
        (() => {
          const quotaItems = parseQuotaItems(usageAccount.quotaRaw);
          const dosageText = getDosageText(usageAccount);
          const planBadge = getPlanBadge(usageAccount);
          return (
            <div className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
              <div className="w-[460px] max-w-[95vw] max-h-[85vh] overflow-y-auto rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5">
                <div className="flex items-center gap-2.5 mb-4">
                  <div className="w-9 h-9 rounded-xl bg-[var(--module-accent)]/15 border border-[var(--module-accent)]/30 flex items-center justify-center">
                    <Gauge className="w-4 h-4 text-[var(--module-accent)]" />
                  </div>
                  <div className="flex-1 min-w-0">
                    <h3 className="text-sm font-bold text-white truncate">{displayName(usageAccount)}</h3>
                    <p className="text-[10px] text-slate-500 truncate">{usageAccount.email}</p>
                  </div>
                  {planBadge !== "UNKNOWN" && (
                    <span className="inline-flex items-center text-[9px] px-1.5 py-0.5 rounded bg-slate-500/15 text-slate-300 border border-slate-500/25 flex-shrink-0">
                      {t(`buddy.plan.${planBadge.toLowerCase()}`, { defaultValue: planBadge })}
                    </span>
                  )}
                  <button
                    onClick={() => setUsageAccountId(null)}
                    className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 cursor-pointer flex-shrink-0"
                  >
                    <X className="w-4 h-4" />
                  </button>
                </div>

                {quotaItems.length === 0 ? (
                  <div className="text-[11px] text-slate-600 mb-3">
                    {usageAccount.quotaQueryLastError
                      ? `${t("buddy.usageError")}: ${usageAccount.quotaQueryLastError}`
                      : t("buddy.noUsage")}
                  </div>
                ) : (
                  <div className="space-y-2.5 mb-3">
                    {quotaItems.map((item, idx) => {
                      const pct =
                        item.total > 0 && !item.unlimited
                          ? Math.max(0, Math.min(100, (item.used / item.total) * 100))
                          : null;
                      return (
                        <div key={idx}>
                          <div className="flex items-center justify-between text-[10px] mb-1">
                            <span className="text-slate-300 truncate">
                              {item.packageName || t("buddy.quota.package")}
                            </span>
                            {item.unlimited ? (
                              <span className="text-emerald-400">∞ {t("buddy.quota.unlimited")}</span>
                            ) : (
                              <span className="text-slate-400 tabular-nums">
                                {formatQuotaNumber(item.used)} / {formatQuotaNumber(item.total)}
                                {pct !== null ? ` (${pct.toFixed(1)}%)` : ""}
                              </span>
                            )}
                          </div>
                          {!item.unlimited && (
                            <div className="h-1.5 bg-white/10 rounded-full overflow-hidden">
                              <div
                                className={`h-full rounded-full ${
                                  pct === null
                                    ? "bg-slate-500/50"
                                    : pct >= 90
                                      ? "bg-rose-500"
                                      : pct >= 60
                                        ? "bg-amber-500"
                                        : "bg-emerald-500"
                                }`}
                                style={{ width: `${pct ?? 0}%` }}
                              />
                            </div>
                          )}
                          {item.cycleEndTime && (
                            <div className="text-[9px] text-slate-600 mt-0.5">
                              {t("buddy.quota.cycleEnd")}: {item.cycleEndTime}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}

                {dosageText && <div className="text-[10px] text-slate-400 mb-1">{dosageText}</div>}
                {usageAccount.paymentType && (
                  <div className="text-[9px] text-slate-600 mb-1">
                    {t("buddy.paymentType")}: {usageAccount.paymentType}
                  </div>
                )}
                {usageAccount.usageUpdatedAt ? (
                  <div className="text-[9px] text-slate-600 mb-3">
                    {t("buddy.usageUpdatedAt")}: {formatTime(usageAccount.usageUpdatedAt)}
                  </div>
                ) : null}

                <div className="flex justify-end gap-2 pt-1">
                  <button
                    onClick={() => refreshAccount(usageAccount.id)}
                    disabled={busy}
                    className="px-3 py-1.5 rounded-lg text-[11px] bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                  >
                    {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <RefreshCw className="w-3 h-3" />}
                    {t("buddy.refresh")}
                  </button>
                  <button
                    onClick={() => setUsageAccountId(null)}
                    className="px-4 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold cursor-pointer"
                  >
                    {t("buddy.close")}
                  </button>
                </div>
              </div>
            </div>
          );
        })()}

      {/* 删除确认（单个删除 / 批量删除共用） */}
      {deleteIds && (
        <div className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
          <div className="w-[360px] max-w-[95vw] rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5">
            <div className="flex items-center gap-2.5 mb-4">
              <div className="w-9 h-9 rounded-xl bg-rose-500/15 border border-rose-500/30 flex items-center justify-center">
                <Trash2 className="w-4 h-4 text-rose-400" />
              </div>
              <div className="flex-1">
                <h3 className="text-sm font-bold text-white">{t("buddy.delTitle")}</h3>
                <p className="text-[10px] text-slate-500">{t("buddy.delHint")}</p>
              </div>
              <button onClick={() => setDeleteIds(null)} className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 cursor-pointer">
                ✕
              </button>
            </div>
            <p className="text-xs text-slate-300 leading-relaxed mb-5">
              {deleteIds.length === 1
                ? (() => {
                    const target = accounts.find((a) => a.id === deleteIds[0]);
                    return t("buddy.delMsgSingle", {
                      name: target ? displayName(target) : deleteIds[0],
                    });
                  })()
                : t("buddy.delMsg", { count: deleteIds.length, platform: platformLabel })}
            </p>
            <div className="flex justify-end gap-2">
              <button onClick={() => setDeleteIds(null)} className="px-3 py-1.5 rounded-lg text-[11px] text-slate-400 hover:bg-white/5 cursor-pointer">
                {t("buddy.cancel")}
              </button>
              <button
                onClick={() => void deleteAccounts(deleteIds)}
                disabled={busy}
                className="px-4 py-1.5 rounded-lg text-[11px] bg-rose-600 hover:bg-rose-500 text-white font-semibold cursor-pointer disabled:opacity-50"
              >
                {t("buddy.deleteConfirm")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 会话删除确认 */}
      {sessionDelete && (
        <div className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
          <div className="w-[360px] max-w-[95vw] rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5">
            <div className="flex items-center gap-2.5 mb-4">
              <div className="w-9 h-9 rounded-xl bg-rose-500/15 border border-rose-500/30 flex items-center justify-center">
                <Trash2 className="w-4 h-4 text-rose-400" />
              </div>
              <div className="flex-1">
                <h3 className="text-sm font-bold text-white">{t("buddy.sessions.deleteTitle")}</h3>
              </div>
              <button onClick={() => setSessionDelete(null)} className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 cursor-pointer">
                ✕
              </button>
            </div>
            <p className="text-xs text-slate-300 leading-relaxed mb-5 break-all">
              {sessionDelete.ids.length === 1
                ? t("buddy.sessions.deleteConfirm", { title: sessionDelete.label })
                : t("buddy.sessions.deleteConfirmBatch", { count: sessionDelete.ids.length })}
            </p>
            <div className="flex justify-end gap-2">
              <button onClick={() => setSessionDelete(null)} className="px-3 py-1.5 rounded-lg text-[11px] text-slate-400 hover:bg-white/5 cursor-pointer">
                {t("buddy.cancel")}
              </button>
              <button
                onClick={() => void deleteSessionsByIds(sessionDelete.ids)}
                disabled={sessionsBusy}
                className="px-4 py-1.5 rounded-lg text-[11px] bg-rose-600 hover:bg-rose-500 text-white font-semibold cursor-pointer disabled:opacity-50"
              >
                {t("buddy.deleteConfirm")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 新增账号弹窗 */}
      {showAdd && (
        <div className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
          <div className="w-[420px] max-w-[95vw] rounded-2xl border border-white/10 bg-slate-900/95 shadow-2xl p-5">
            <div className="flex items-center gap-2.5 mb-4">
              <div className="w-9 h-9 rounded-xl bg-[var(--module-accent)]/15 border border-[var(--module-accent)]/30 flex items-center justify-center">
                {addMode === "oauth" ? <LogIn className="w-4 h-4 text-[var(--module-accent)]" /> : addMode === "token" ? <KeyRound className="w-4 h-4 text-[var(--module-accent)]" /> : <Download className="w-4 h-4 text-[var(--module-accent)]" />}
              </div>
              <div className="flex-1">
                <h3 className="text-sm font-bold text-white">{t("buddy.addTitle", { platform: platformLabel })}</h3>
              </div>
              <button onClick={() => void cancelOAuth()} className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 cursor-pointer">
                <X className="w-4 h-4" />
              </button>
            </div>

            {/* 模式切换 */}
            <div className="flex items-center gap-1 bg-black/30 rounded-lg border border-white/10 p-0.5 mb-4">
              <button
                onClick={() => setAddMode("oauth")}
                className={`flex-1 px-2 py-1.5 rounded-md text-[11px] transition cursor-pointer ${
                  addMode === "oauth" ? "bg-[var(--module-accent)]/25 text-white font-semibold" : "text-slate-400"
                }`}
              >
                {t("buddy.addOAuth")}
              </button>
              <button
                onClick={() => setAddMode("token")}
                className={`flex-1 px-2 py-1.5 rounded-md text-[11px] transition cursor-pointer ${
                  addMode === "token" ? "bg-[var(--module-accent)]/25 text-white font-semibold" : "text-slate-400"
                }`}
              >
                {t("buddy.addToken")}
              </button>
              <button
                onClick={() => setAddMode("local")}
                className={`flex-1 px-2 py-1.5 rounded-md text-[11px] transition cursor-pointer ${
                  addMode === "local" ? "bg-[var(--module-accent)]/25 text-white font-semibold" : "text-slate-400"
                }`}
              >
                {t("buddy.importLocal")}
              </button>
            </div>

            {addMode === "oauth" ? (
              oauth ? (
                <div className="space-y-3">
                  <p className="text-[11px] text-slate-400 leading-relaxed">{t("buddy.oauthWaitHint")}</p>
                  <a
                    href={oauth.verificationUri}
                    target="_blank"
                    rel="noreferrer"
                    className="flex items-center gap-2 text-[11px] text-[var(--module-accent)] hover:underline break-all"
                  >
                    <ExternalLink className="w-3 h-3 flex-shrink-0" />
                    {oauth.verificationUri}
                  </a>
                  <p className="text-[9px] text-slate-600">
                    {t("buddy.oauthExpires", { minutes: Math.round(oauth.expiresIn / 60) })}
                  </p>
                  <div className="flex justify-end gap-2 pt-2">
                    <button
                      onClick={() => void cancelOAuth()}
                      className="px-3 py-1.5 rounded-lg text-[11px] text-slate-400 hover:bg-white/5 cursor-pointer"
                    >
                      {t("buddy.cancel")}
                    </button>
                    <button
                      onClick={() => void completeOAuth()}
                      disabled={oauthBusy}
                      className="px-4 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                    >
                      {oauthBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
                      {t("buddy.oauthComplete")}
                    </button>
                  </div>
                </div>
              ) : (
                <div className="space-y-3">
                  <p className="text-[11px] text-slate-400 leading-relaxed">{t("buddy.oauthStartHint")}</p>
                  <button
                    onClick={startOAuth}
                    disabled={oauthBusy}
                    className="w-full px-3 py-2 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold flex items-center justify-center gap-1 cursor-pointer transition disabled:opacity-50"
                  >
                    {oauthBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <LogIn className="w-3 h-3" />}
                    {t("buddy.oauthStart")}
                  </button>
                </div>
              )
            ) : addMode === "token" ? (
              <div className="space-y-3">
                <textarea
                  value={tokenInput}
                  onChange={(e) => setTokenInput(e.target.value)}
                  placeholder={t("buddy.tokenPlaceholder")}
                  rows={4}
                  className="w-full bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-[11px] text-white outline-none focus:border-[var(--module-accent)]/50 placeholder:text-slate-600 resize-none"
                />
                <div className="flex justify-end gap-2">
                  <button
                    onClick={() => setShowAdd(false)}
                    className="px-3 py-1.5 rounded-lg text-[11px] text-slate-400 hover:bg-white/5 cursor-pointer"
                  >
                    {t("buddy.cancel")}
                  </button>
                  <button
                    onClick={() => void addWithToken()}
                    disabled={busy || !tokenInput.trim()}
                    className="px-4 py-1.5 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
                  >
                    <KeyRound className="w-3 h-3" /> {t("buddy.addTokenSubmit")}
                  </button>
                </div>
              </div>
            ) : (
              <div className="space-y-3">
                <p className="text-[11px] text-slate-400 leading-relaxed">{t("buddy.importLocalHint")}</p>
                <button
                  onClick={() => void importFromLocal()}
                  disabled={busy}
                  className="w-full px-3 py-2 rounded-lg text-[11px] bg-[var(--module-accent)] hover:opacity-85 text-white font-semibold flex items-center justify-center gap-1 cursor-pointer transition disabled:opacity-50"
                >
                  {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Download className="w-3 h-3" />}
                  {t("buddy.importLocal")}
                </button>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}