// 音乐播放器前后端共享类型（与 Rust 侧 serde 结构一一对应）。

/**
 * 播放模式。推进逻辑（顺序/随机洗牌/单曲循环）在后端
 * （src-tauri/src/commands/music/queue.rs），这里只保留类型供界面使用。
 */
export type PlayMode = "sequence" | "shuffle" | "single";

/** 一首曲目（来自 data_dir/music/library.json 的缓存） */
export interface MusicTrack {
  path: string;
  title: string;
  artist: string;
  album: string;
  duration_ms: number;
  size_bytes: number;
  /** 所属导入目录 */
  folder: string;
}

export interface MusicLibrary {
  folders: string[];
  tracks: MusicTrack[];
}

export type PlayerStatus = "idle" | "playing" | "paused" | "ended";

export interface PlayerState {
  status: PlayerStatus;
  path: string | null;
  title: string | null;
  artist: string | null;
  position_ms: number;
  duration_ms: number;
  volume: number;
  /** 当前曲目来自在线音源时的来源（本地曲目为 null） */
  online: OnlineMeta | null;
}

/** 在线来源摘要：够显示「下载」按钮与默认音质，不含曲目对象 */
export interface OnlineMeta {
  /** 来源插件展示名 */
  platform: string;
  /** 当前缓存所用音质 */
  quality: MusicQuality;
}

/** 音质档位（顺序与后端一致） */
export const MUSIC_QUALITIES: MusicQuality[] = ["low", "standard", "high", "super"];

export interface EqParams {
  enabled: boolean;
  bands: number[];
  gain_db: number;
  balance: number;
  preset: string;
}

export interface MusicSettings {
  volume: number;
  play_mode: PlayMode;
  eq: EqParams;
  /** 在线音源下载目录；空串 = 默认（数据目录/music/downloads） */
  download_dir: string;
}

export interface EqPresetInfo {
  id: string;
  bands: number[];
}

export interface AddFolderResult {
  added: number;
  library: MusicLibrary;
}

/** 重命名弹窗的预填建议（后端 music_track_name_suggestion） */
export interface TrackNameSuggestion {
  /** 磁盘上的现名（含扩展名） */
  current_name: string;
  /** 推荐新名；无可解析标签时回退为现名 */
  suggested_name: string;
  /** 建议是否来自音频标签 */
  from_tags: boolean;
}

export interface RenameTrackResult {
  old_path: string;
  new_path: string;
  /** 文件名是否真的变了（同名提交时为 false） */
  renamed: boolean;
  library: MusicLibrary;
}

export interface DeleteTracksResult {
  deleted: number;
  /** 失败项（`路径（原因）`） */
  failed: string[];
  library: MusicLibrary;
  /** 删到正在播放那首时会自动切歌，这里带回切换后的播放状态 */
  player: PlayerState;
}

/** 内置曲线（后端 music_list_builtin_curves）：原文给前端，套用走同一条解析路径 */
export interface BuiltinCurve {
  /** 稳定 id，文案键为 `music.eqBuiltin.<id>` */
  id: string;
  /** GraphicEQ 文本 */
  text: string;
}

/** 曲线导入结果（后端 music_parse_eq_curve） */
export interface EqCurveImport {
  /** 折叠到 10 段后的增益（dB） */
  bands: number[];
  /** 解析出的原始频点数量 */
  points: number;
  min_freq: number;
  max_freq: number;
}

/* ─────────────── 在线音源（MusicFree 插件）───────────────
 * 全部与 Rust 侧 serde 结构一一对应（字段名即后端字段名，snake_case）。
 */

/** 音质（插件 `getMediaSource` 的入参） */
export type MusicQuality = "low" | "standard" | "high" | "super";

/** 插件自述：导入时探测一次并缓存，列表页不必启 Node（见后端 plugin_registry） */
export interface PluginMeta {
  platform: string;
  version: string;
  author: string;
  description: string;
  /** 插件自带的更新地址 */
  src_url: string;
  has_search: boolean;
  has_media_source: boolean;
  supported_search_type: string[];
  /** 插件的自定义输入项，原样透传（当前未使用） */
  user_variables: unknown[];
}

export interface PluginEntry {
  /** scripts/ 下的文件名，作为稳定 id */
  file: string;
  /** 用户改过的名字（空则回退自述的 platform，再回退文件名） */
  name: string;
  enabled: boolean;
  order: number;
  /** 导入来源（URL / 本地路径）；空 = 用户自己放进插件目录的 */
  source: string;
  imported_at: string;
  meta: PluginMeta;
}

/** 插件沙箱（Node 权限模型）状态 */
export interface SandboxStatus {
  /** 是否启用了 `--permission` */
  enabled: boolean;
  /** 未启用时的原因（如 Node 版本过低） */
  reason: string | null;
}

/** 依赖现状：插件依赖是**功能级前置**，一次装、所有插件共享 */
export interface DepsReport {
  ready: boolean;
  node_found: boolean;
  node_path: string | null;
  npm_path: string | null;
  missing_packages: string[];
  root: string;
  /** 环境问题（缺 Node 等）；有值时 ready 必为 false */
  problem: string | null;
  sandbox: SandboxStatus;
}

export interface PluginListResult {
  deps: DepsReport;
  plugins: PluginEntry[];
  dir: string;
}

export interface PluginFailure {
  file: string;
  name: string;
  error: string;
}

/**
 * 导入结果：后端按**内容**自动识别是「单个插件」还是「订阅清单」。
 *
 * 之所以不按扩展名/按钮分派：两者的来源形态完全一样（都可能是 URL，也都可以是本地文件），
 * 按扩展名分一定会漏 —— 官方订阅列表本身就是个 `.json` URL。
 */
export type PluginImportResult =
  | {
      kind: "single";
      file: string;
      source: string;
      /** 界面显示名（订阅清单里给了名字时用订阅的名字） */
      name: string;
      meta: PluginMeta;
      /** 落盘成功但探测信息失败的原因；插件仍可用 */
      probe_error: string | null;
    }
  | {
      kind: "subscription";
      plugins: PluginEntry[];
      imported: number;
      failures: PluginFailure[];
    };

/** 插件返回的原始曲目对象（原样透传，回传给 getMediaSource 时不能裁剪） */
export interface OnlineTrack {
  id?: string;
  title?: string;
  artist?: string;
  album?: string;
  /** 秒 */
  duration?: number;
  artwork?: string;
  [key: string]: unknown;
}

export interface SearchHit {
  /** 来源插件文件名 */
  file: string;
  /** 来源插件显示名 */
  platform: string;
  item: OnlineTrack;
}

export interface SearchOutcome {
  hits: SearchHit[];
  /** 已到末页的插件 */
  exhausted: string[];
  /** 失败的插件：单个音源出错不影响其它音源的结果 */
  failures: PluginFailure[];
}

export interface ResolvedSource {
  url: string;
  /** 插件要求的请求头（防盗链等），需原样带上 */
  headers: [string, string][];
  user_agent: string;
  quality: string;
}

export interface PluginPlayOutcome {
  /** 实际播放的本地缓存文件 */
  path: string;
  /** 是否命中缓存（命中时没走网络） */
  from_cache: boolean;
  title: string;
  artist: string;
  /** 命中缓存时为 null */
  source: ResolvedSource | null;
  player: PlayerState;
}

export interface PluginDownloadOutcome {
  path: string;
  title: string;
  artist: string;
  bytes: number;
  source: ResolvedSource;
  dir: string;
  library: MusicLibrary;
}

/** 下载进度事件负载（music-plugin-download-progress） */
export interface DownloadProgress {
  label: string;
  received: number;
  total: number;
  done?: boolean;
}

export interface PluginStorageInfo {
  download_dir: string;
  cache_dir: string;
  cache_bytes: number;
}

/** 插件下载目录已在数据库中登记：把字节数格式化成可读文本 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

/** 秒 → mm:ss（插件给的时长是秒，不是毫秒） */
export function formatSeconds(seconds: number | undefined): string {
  if (!seconds || !Number.isFinite(seconds) || seconds <= 0) return "--:--";
  return formatTime(seconds * 1000);
}

/** 毫秒 → mm:ss（超过一小时显示 h:mm:ss） */
export function formatTime(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "00:00";
  const total = Math.floor(ms / 1000);
  const seconds = total % 60;
  const minutes = Math.floor(total / 60) % 60;
  const hours = Math.floor(total / 3600);
  const pad = (n: number) => String(n).padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${pad(minutes)}:${pad(seconds)}`;
}

/** 取路径最后一段作为文件夹名 */
export function folderName(path: string): string {
  const parts = path.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || path;
}
