import { LABEL_FIELDS, REMAINING_FIELDS, TOTAL_FIELDS, USED_FIELDS, mergeCreditSegments, pickNumber, pickText, segmentsFromQuotaItems, summarizeCreditSegments } from "./quota";
import { formatCountdown } from "./timeLabel";
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
  /** 令牌签发域（JWT `iss` 的 origin，如 `https://www.codebuddy.cn`）；解不出时为 null */
  authOrigin?: string | null;
  /** 该平台客户端鉴权时使用的域 */
  platformAuthOrigin?: string | null;
  /** 签发域是否与客户端域一致；null = 解不出（不拦，交给用户判断） */
  authOriginMatches?: boolean | null;
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

/** 单个会话在一次同步中的结局（后端 session_sync::SessionSyncStatus） */
export type BuddySessionSyncStatus =
  | "copied"
  | "skipped"
  | "partial"
  | "conflict"
  | "failed";

export interface BuddySessionSyncDetail {
  id: string;
  label: string;
  /** 该会话所属目录 / 项目（后端取不到时为 null，界面显示占位符） */
  workspace: string | null;
  status: BuddySessionSyncStatus;
  /** 机器可读原因码，前端用 buddy.syncReason.<reason> 翻译 */
  reason: string;
}

/** 「只处理有变化的会话」台账（后端 session_sync::SessionSyncSummary） */
export interface BuddySessionSyncSummary {
  total: number;
  copied: number;
  skipped: number;
  partial: number;
  conflict: number;
  failed: number;
  /** 全部会话都无变化 */
  unchanged: boolean;
  details: BuddySessionSyncDetail[];
}

export interface BuddyTransferReport {
  addedConversations: number;
  replacedConversations: number;
  updatedSessionRows: number;
  scannedWorkspaces: number;
  sync?: BuddySessionSyncSummary;
}

/** 待用户裁决的会话冲突（后端 session_sync::PendingConflict，切换时两侧都改过的会话） */
export interface BuddyPendingConflict {
  id: string;
  /** 展示名：标题优先，回落到目录 —— 不会是裸 id */
  label: string;
  /** 所属工作区目录（CodeBuddy 侧为 MD5 哈希名，定位文件用） */
  workspace: string;
  /** 哈希还原出的真实项目路径；还原失败为 null，展示时回退到 workspace */
  workspacePath?: string | null;
  /** IDE 目录名 */
  ide: string;
  sourceUid: string;
  targetUid: string;
  /** 双方最后消息时间（epoch 毫秒） */
  sourceStamp: number;
  targetStamp: number;
}

/** 用户对冲突的裁决方式：merge 取较新 / overwrite 用来源覆盖目标 / keep 保留目标 / ignore 不处理只结束 */
export type ConflictAction = "merge" | "overwrite" | "keep" | "ignore";

/** 一条冲突的处理结果（后端 session_sync::ConflictResolution） */
export interface BuddyConflictResolution {
  id: string;
  /** merge | overwrite | keep */
  action: string;
  /** 处理时间（epoch 毫秒） */
  atMs: number;
  /** 后端给的人类可读结果 */
  message: string;
}

/** 冲突会话一侧的对话消息预览（后端已从双层 JSON 抽出正文） */
export interface BuddyConflictMessage {
  role: string;
  time: string;
  text: string;
}

/** 同步明细里各状态的颜色（与 buddy.syncStatus.* 一一对应） */
export const SYNC_STATUS_CLASS: Record<string, string> = {
  copied: "text-emerald-400",
  skipped: "text-slate-400",
  partial: "text-amber-400",
  conflict: "text-amber-400",
  failed: "text-rose-400",
};

// ─── 成长计划 / Buddy 状态（后端 buddy_growth_overview，仅 WorkBuddy） ───

export interface BuddyGrowthTask {
  taskCode: string;
  title: string;
  guide: string;
  tag: string;
  deadline?: string | null;
  reward: { credits: number; energy: number; buddy: boolean };
  current: number;
  target: number;
  state: "claimed" | "completed" | "not_accepted" | "in_progress" | string;
}

export interface BuddyGrowthOverview {
  fetchedAt: number;
  buddyKnown: boolean;
  unlocked: boolean;
  growth: { completed: number; total: number; ratio: number; tasks: BuddyGrowthTask[] };
  rewards: { claimed: number; total: number; pending: number; ratio: number };
  cat: {
    state: string;
    progress: number;
    arriveAt?: number | null;
    dailyLimitReached: boolean;
    available: boolean;
    activeBuddy: boolean;
    buddies: { instanceId: number; name: string }[];
    rewardCredits: number;
    rewardEnergy: number;
  };
  actions: {
    gachaAvailable: boolean;
    gachaCount?: number | null;
    gachaEnergy?: number | null;
    gachaCost?: number | null;
    lotteryAvailable: boolean;
    lotteryCount?: number | null;
  };
  manualTasks: string[];
  streak?: {
    days?: number | null;
    progressDays: number;
    nextTier?: string | null;
    nextTierRemaining?: number | null;
    makeupCards?: number | null;
    tiers: { key: string; days: number; status: string }[];
    status: string;
  } | null;
  warnings: string[];
}

/** 猫猫旅行状态 → i18n 键（未知状态回落到官方原文） */
export const GROWTH_CAT_STATE_KEY: Record<string, string> = {
  locked: "buddy.growth.travelLocked",
  unknown: "buddy.growth.travelUnknown",
  needs_selection: "buddy.growth.travelNeedsSelection",
  idle: "buddy.growth.travelIdle",
  traveling: "buddy.growth.travelTraveling",
  arrived: "buddy.growth.travelArrived",
};

export interface BuddyClientPath {
  platform: string;
  label: string;
  configured?: string | null;
  resolved?: string | null;
}

export interface BuddySwitchProgress {
  platform: string;
  accountId: string;
  stage: "checking" | "merging" | "writing" | "done";
  scannedWorkspaces: number;
  message?: string | null;
  /** 合并结束时后端一次性带上的同步台账 */
  sync?: BuddySessionSyncSummary;
}

/** 冲突裁决进度（后端 buddy-conflict-progress 事件）。
 *  裁决要递归复制整个会话目录，批量跑得久 —— 没有它用户只能看到一个转圈的按钮。 */
export interface BuddyConflictProgress {
  platform: string;
  stage: "resolving" | "done";
  total: number;
  done: number;
  currentId?: string | null;
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
  /** 活动时间：last_activity_at → updated_at → created_at 回退（后端已算好） */
  lastActivityAt?: number | null;
  isPlayground: boolean;
  /** 本地已找不到该会话正文（后端探测；定位不到目录布局时不返回 true） */
  contentMissing?: boolean;
  locations: { instanceId: string; instanceName: string }[];
}

/** 后端 buddy_delete_sessions 的删除结果 */
/** 会话分叉结果：新会话 id + 正文复制情况 */
export interface BuddySessionForkReport {
  conversationId: string;
  title: string;
  filesCopied: number;
  /** 正文一个都没拷到（只有库记录被复制） */
  contentMissing: boolean;
  errors: string[];
}

/** 改会话目录的结果（后端搬完正文 + 索引后返回） */
export interface BuddySessionMoveReport {
  moved: number;
  /** 搬动的目录数（正文 + 各类辅助目录） */
  dirsMoved: number;
  /** 没动成的会话：`id：原因` */
  skipped: string[];
  /** 本次备份根目录（要还原就把它整棵放回去） */
  backupDir: string;
}

export interface BuddySessionDeleteReport {
  dbDeleted: number;
  historyDirsRemoved: number;
  auxiliaryDirsRemoved: number;
  /** WorkBuddy 5.x 删除的正文项数（jsonl / 会话目录 / 快照 / 文件历史 / artifact 索引） */
  filesRemoved: number;
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
  /** unfinished：计划时间已错过且今天的窗口已结束（今日放弃，等明天重新随机安排） */
  status: "pending" | "success" | "failed" | "unfinished";
  scheduledTime?: string | null;
  lastAttemptTime?: string | null;
  /** 实际签到时刻（HH:MM:SS，当日已签到时存在） */
  lastCheckinTime?: string | null;
  /** 实际时间来源（kira），旧记录可能为空 */
  source?: string | null;
  message?: string | null;
}

export interface BuddyCheckinTasksView {
  enabled: boolean;
  startTime: string;
  endTime: string;
  /** 今日计划是否已生成 */
  generated: boolean;
  tasks: BuddyCheckinTask[];
}

/** 单个账号的今日派旅行任务（后端 buddy_auto_travel_tasks 返回） */
export interface BuddyTravelTask {
  accountId: string;
  email: string;
  /**
   * pending（待派出）| traveling（旅行中）| arrived（已归来待领取）| claimed（已领取）
   * | limit_reached（今日已结束）| rejected（派出被拒）| failed（失败）
   * | unfinished（当天未完成）| none（未安排）
   */
  status: string;
  /** 计划派出时间（HH:MM） */
  planTime?: string | null;
  /** 实际派出时间（HH:MM:SS） */
  departTime?: string | null;
  /** 归来时间（HH:MM:SS，轮询发现的时刻） */
  backTime?: string | null;
  /** 领取奖励时间（HH:MM:SS） */
  claimTime?: string | null;
  /** 今日派出次数（每天从 0 开始） */
  departCount: number;
  credit?: number | null;
  message?: string | null;
}

export interface BuddyTravelTasksView {
  enabled: boolean;
  startTime: string;
  endTime: string;
  locationId: number;
  generated: boolean;
  tasks: BuddyTravelTask[];
}

/** 每日归档：单账号某天的签到记录 */
export interface BuddyDailyCheckinRecord {
  planTime?: string | null;
  actualTime?: string | null;
  status: string;
  source?: string | null;
  credit?: number | null;
  streak?: number | null;
  message?: string | null;
}

/** 每日归档：单账号某天的派旅行记录 */
export interface BuddyDailyTravelRecord {
  planTime?: string | null;
  departTime?: string | null;
  backTime?: string | null;
  claimTime?: string | null;
  departCount: number;
  status: string;
  credit?: number | null;
  message?: string | null;
}

export interface BuddyDailyAccountRecord {
  email: string;
  checkin?: BuddyDailyCheckinRecord | null;
  travel?: BuddyDailyTravelRecord | null;
}

/** 每日归档：日期 → 账号 id → 记录（buddy_get_daily_records 返回） */
export type BuddyDailyRecords = Record<string, Record<string, BuddyDailyAccountRecord>>;

export interface CheckinStatus {
  todayCheckedIn: boolean;
  active: boolean;
  streakDays: number;
  dailyCredit: number;
}

/** 账号视图偏好（后端 `<platform>_view.json`）：主账号 + 排序模式 */
export interface BuddyAccountView {
  primaryAccountId?: string | null;
  orderMode: "lastUsed" | "quota" | "expiry" | string;
}

export interface OAuthStartResponse {
  loginId: string;
  verificationUri: string;
  verificationUriComplete?: string | null;
  expiresIn: number;
  intervalSeconds: number;
}

export interface QuotaItem {
  packageName: string;
  used: number;
  total: number;
  remain: number;
  unlimited: boolean;
  cycleEndTime?: string | null;
  /** 官方礼包码：同一礼包的多条记录据此合并 */
  packageCode?: string;
}

export const PLATFORMS = [
  { id: "codebuddy-cn", label: "CodeBuddy CN", emoji: "🇨🇳" },
  { id: "workbuddy", label: "WorkBuddy", emoji: "🟣" },
] as const;

export type Tab = "accounts" | "twoapi" | "sessions" | "checkin" | "settings";

/** 派 Buddy 旅行可选地点（与后端 location_id 对应） */
export const TRAVEL_LOCATIONS = [
  { id: 1, key: "loc1" },
  { id: 2, key: "loc2" },
  { id: 3, key: "loc3" },
  { id: 4, key: "loc4" },
] as const;

export const localDateStr = () => {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(
    d.getDate()
  ).padStart(2, "0")}`;
};

/** 当前月份 "YYYY-MM" */
export const localMonthStr = () => {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
};

/** 月份 → 日历网格（周日开头，月初补空格），元素为 "YYYY-MM-DD" 或 null */
export const buildMonthCells = (month: string): (string | null)[] => {
  const [year, m] = month.split("-").map(Number);
  if (!year || !m) return [];
  const lead = new Date(year, m - 1, 1).getDay();
  const daysInMonth = new Date(year, m, 0).getDate();
  const cells: (string | null)[] = Array.from({ length: lead }, () => null);
  for (let day = 1; day <= daysInMonth; day += 1) {
    cells.push(`${month}-${String(day).padStart(2, "0")}`);
  }
  return cells;
};

/** 月份 → [月初, 月末]（供 buddy_get_daily_records 查询） */
export const monthBounds = (month: string): { from: string; to: string } => {
  const [year, m] = month.split("-").map(Number);
  const daysInMonth = new Date(year, m, 0).getDate();
  return { from: `${month}-01`, to: `${month}-${String(daysInMonth).padStart(2, "0")}` };
};

/** 某月前后移动 N 个月 */
export const shiftMonthStr = (month: string, delta: number): string => {
  const [year, m] = month.split("-").map(Number);
  const d = new Date(year, m - 1 + delta, 1);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
};

/** 单日完成度：签到完成数 / 派出完成数 / 账号总数（日历格子与明细用） */
export const dayProgress = (records: Record<string, BuddyDailyAccountRecord> | undefined) => {
  const list = Object.values(records ?? {});
  let checkin = 0;
  let travel = 0;
  for (const record of list) {
    const checkinStatus = record.checkin?.status;
    if (checkinStatus === "success" || checkinStatus === "already_checked") checkin += 1;
    const travelStatus = record.travel?.status;
    if (travelStatus === "claimed" || travelStatus === "limit_reached") travel += 1;
  }
  return { total: list.length, checkin, travel };
};

export const fmtMinute = (m: number) =>
  `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;

/** "HH:MM:SS" 加 N 分钟 → "HH:MM" */
export const addMinutes = (time: string, minutes: number) => {
  const [h, m] = time.split(":").map(Number);
  if (Number.isNaN(h) || Number.isNaN(m)) return time;
  return fmtMinute((h * 60 + m + minutes) % 1440);
};

// ─── 用量解析 ───

export function parseQuotaItems(quotaRaw: unknown): QuotaItem[] {
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
      return {
        // 字段别名表集中在 quota.ts 维护（官方历史上改过多次字段名）
        packageName: pickText(r, LABEL_FIELDS) || String(r.packageName ?? ""),
        used: pickNumber(r, USED_FIELDS) ?? 0,
        total: pickNumber(r, TOTAL_FIELDS) ?? 0,
        remain: pickNumber(r, REMAINING_FIELDS) ?? 0,
        unlimited: Boolean(r.Unlimited),
        packageCode: String(r.PackageCode ?? r.packageCode ?? ""),
        // 到期时间原样交给 UI 展示（别名表里的时间字段由 quota.ts 的 pickTimestamp 处理）
        cycleEndTime:
          (r.CycleEndTime as string | undefined) ??
          (r.cycleEndTime as string | undefined) ??
          (r.EndTime as string | undefined) ??
          (r.PackageEndTime as string | undefined) ??
          null,
      };
    })
    .filter((item) => item.total > 0 || item.unlimited || item.packageName);
}

/** 某账号的积分段（用于「积分不足 → 换号建议」）。 */
export function accountCreditSegments(account: BuddyAccount) {
  return segmentsFromQuotaItems(parseQuotaItems(account.quotaRaw));
}

export function getDosageText(account: BuddyAccount): string | null {
  const zh = account.dosageNotifyZh?.trim();
  const en = account.dosageNotifyEn?.trim();
  const lang = navigator.language?.toLowerCase() ?? "";
  const text = lang.startsWith("zh") ? zh : en;
  return text || zh || en || null;
}

export function getPlanBadge(account: BuddyAccount): string {
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

// 总额度（不区分个人体验/裂变包，全部合并为一个 remain/total）
// 同一礼包（礼包码/来源 + 到期时间）的多条记录先合并再求和，与官方礼包分组一致；
// 单条记录 total < remain 时以 remain 为准（对齐参考实现的归一化）。
export function summarizeQuota(items: QuotaItem[]): {
  used: number;
  total: number;
  remain: number;
  unlimited: boolean;
  hasData: boolean;
} {
  let used = 0;
  let unlimited = false;
  const billable: QuotaItem[] = [];
  for (const it of items) {
    if (it.unlimited) {
      unlimited = true;
      continue;
    }
    used += it.used;
    billable.push(it);
  }
  const merged = summarizeCreditSegments(
    mergeCreditSegments(segmentsFromQuotaItems(billable))
  );
  return {
    used,
    total: merged.total,
    remain: merged.remain,
    unlimited,
    hasData: items.length > 0,
  };
}

// 恢复进度条的填充窗口：限额窗口多为小时级，取最后 6h 由 0→100%
export const RECOVER_HORIZON_MS = 6 * 3600000;

export interface LabelState {
  // true = 已过恢复时刻（额度已恢复、可用）；false = 仍在等待恢复（倒计时）
  recovered: boolean;
  progress: number;
  tone: "sky" | "amber" | "emerald";
  text: string;
  absolute: string;
}

export function describeLabel(ts: number, now: number): LabelState {
  const absolute = new Date(ts).toLocaleString();
  const diff = ts - now;
  if (diff <= 0) {
    // 时间标签记录的是"恢复时刻"：此刻已过 → 额度已恢复（绿色，进度满）
    // 文本带负号（`formatCountdown` 对负数加 `-`）：已过多久
    return { recovered: true, progress: 100, tone: "emerald", text: formatCountdown(diff), absolute };
  }
  // 尚未到恢复时刻：进度随临近而增长，最后 30 分钟转琥珀提示"即将恢复"
  const progress = Math.max(0, Math.min(100, Math.round((1 - diff / RECOVER_HORIZON_MS) * 100)));
  const tone: LabelState["tone"] = diff < 30 * 60000 ? "amber" : "sky";
  return { recovered: false, progress, tone, text: formatCountdown(diff), absolute };
}

export function formatTime(t: number | null | undefined): string {
  if (!t) return "—";
  return new Date(t * 1000).toLocaleString();
}

// 宽松解析用户输入/粘贴的时间：自动去空格、统一分隔符，容忍
// "2026-09-10 19:45:01" / "2026/9/10 19:45" / "2026.09.10T19:45" / "2026-09-10"（本地时区）
export function parseTimeInput(raw: string): number | null {
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
export function formatTimeInput(ts: number): string {
  const d = new Date(ts);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

export function formatCwd(cwd: string): string {
  const normalized = cwd.replace(/\\/g, "/").replace(/\/$/, "");
  if (normalized.length <= 50) return normalized;
  const parts = normalized.split("/").filter(Boolean);
  if (parts.length <= 2) return normalized;
  return `/${parts[0]}/.../${parts.slice(-2).join("/")}`;
}

export function formatRelative(t: number | null | undefined): string {
  if (!t) return "—";
  // 单位兼容：WorkBuddy 会话库是毫秒，部分数据源是秒（< 1e12 视为秒）
  const ms = t < 1e12 ? t * 1000 : t;
  const diff = (Date.now() - ms) / 1000;
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

// 账号行右侧小图标按钮统一样式
export const ACC_BTN = "ui-btn px-1.5 py-1 disabled:opacity-40";

// 账号 tab 工具栏行按钮（与「加列」按钮同尺寸）：刷新/导出/导入/删除/全选都在这一行。
// 这些是给 code/work 账号库用的专用操作，不是通用按钮，所以收进 tab 内而不是面板顶栏。
export const TOOL_BTN =
  "inline-flex items-center gap-0.5 px-1.5 py-1 rounded-md border border-white/10 text-slate-400 hover:text-white hover:border-white/25 cursor-pointer transition disabled:opacity-40 disabled:cursor-not-allowed";

export const TOOL_BTN_DANGER =
  "inline-flex items-center gap-0.5 px-1.5 py-1 rounded-md border border-rose-500/20 text-rose-300 hover:text-rose-200 hover:border-rose-500/40 cursor-pointer transition disabled:opacity-40 disabled:cursor-not-allowed";

export const TOOL_BTN_PRIMARY =
  "inline-flex items-center gap-0.5 px-1.5 py-1 rounded-md border border-transparent bg-[var(--module-accent)]/20 text-[var(--module-accent)] hover:bg-[var(--module-accent)]/30 font-semibold cursor-pointer transition disabled:opacity-40 disabled:cursor-not-allowed";

// 账号表格各列固定宽度（px），表头与行按同一宽度对齐
export const COL_INFO = 240;

/** 积分列：构成条（一段一方块，块内显示剩余天数）+ 右侧剩余总额。
    原来「额度」与「积分构成」是两列，但两者说的是同一笔积分，分开后要来回对照才知道
    这些方块一共是多少 —— 合并成一列，总额就放在构成条右边。
    **填充列**：剩余宽度全给它（`flex-1`）—— 段多的账号最需要宽度，窗口越宽构成条越长。
    这里是它的**最小宽度**：窗口窄或自定义列多时靠横向滚动兜底，不能再压。 */
export const COL_CREDITS_MIN = 240;

/** 时间列（内置「到期」+ 每个自定义列）：只要放得下 `55.21` 这种紧凑写法 */
export const COL_W = 72;

/** 操作列：只剩「切换」1 个图标按钮，按实际按钮宽度收死（不再靠 `pl-9` 把按钮顶到右边，
    那样按钮会被挤在一半列宽里）。「用量」不再重复给按钮——点积分构成条即可打开同一个用量弹窗；
    刷新/导出/删除已收进 tab 工具栏行，按勾选批量操作。 */
export const COL_ACTIONS = 32;

// 时间标签 chip 配色（sky=等待恢复 / amber=即将恢复 / emerald=已恢复可用）
export const LABEL_TONE: Record<LabelState["tone"], { chip: string; bar: string; text: string }> = {
  sky: { chip: "bg-sky-500/10 border-sky-500/25", bar: "bg-sky-500/30", text: "text-sky-300" },
  amber: { chip: "bg-amber-500/10 border-amber-500/30", bar: "bg-amber-500/35", text: "text-amber-300" },
  emerald: { chip: "bg-emerald-500/10 border-emerald-500/30", bar: "bg-emerald-500/40", text: "text-emerald-300" },
};

// ─── 会话按项目（工作目录）分组 ───

export interface SessionGroup {
  cwd: string;
  sessions: BuddySessionRecord[];
  latestUpdatedAt: number;
}

export function buildSessionGroups(sessions: BuddySessionRecord[]): SessionGroup[] {
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
        (a, b) => sessionActivityAt(b) - sessionActivityAt(a) || a.title.localeCompare(b.title)
      ),
      latestUpdatedAt: Math.max(...groupSessions.map(sessionActivityAt), 0),
    }))
    .sort((a, b) => b.latestUpdatedAt - a.latestUpdatedAt || a.cwd.localeCompare(b.cwd, "zh-CN"));
}

/** 会话活动时间：后端已按 last_activity_at → updated_at → created_at 回退算好，这里再兜底一次。 */
export function sessionActivityAt(session: BuddySessionRecord): number {
  return session.lastActivityAt ?? session.updatedAt ?? session.createdAt ?? 0;
}

export function resolveGroupLabel(cwd: string): string {
  const normalized = cwd.replace(/\\/g, "/").replace(/\/$/, "");
  const parts = normalized.split("/").filter(Boolean);
  return parts[parts.length - 1] || cwd || "（无目录）";
}
