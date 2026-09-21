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
}

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
