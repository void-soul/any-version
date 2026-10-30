export interface Classification {
  id: number;
  parentId: number | null;
  name: string;
  classificationType: number; // 0: 普通分类, 1: 关联文件夹, 2: 聚合分类
  data: ClassificationData;
  shortcutKey: string | null;
  globalShortcutKey: boolean;
  order: number;
  childList?: Classification[];
  itemCount?: number;
}

/** 删除分类的结果（两种模式共用：级联看 deleted_*，迁移看 moved_*） */
export interface DeleteClassificationResult {
  deletedCategories: number;
  deletedItems: number;
  movedCategories: number;
  movedItems: number;
}

export interface ClassificationData {
  icon?: string | null;
  associateFolderPath?: string | null;
  associateFolderHiddenItems?: string | null;
  itemLayout?: "default" | "tile" | "list" | "compact" | "icon_only";
  itemSort?: "default" | "initial";
  itemColumnNumber?: number | null;
  itemIconSize?: number | null;
  itemShowOnly?: "default" | "file" | "folder";
  fixed?: boolean;
  excludeSearch?: boolean;
  defaultCollapsed?: boolean;
  aggregateItemCount?: number | null;
  aggregateSort?: "openNumber" | "lastOpen" | "name" | null;
}

export interface Item {
  id: number;
  classificationId: number;
  name: string;
  itemType: number; // 0: 文件/应用, 1: 文件夹, 2: 网址, 3: 系统, 4: Appx, 5: 多项目
  data: ItemData;
  shortcutKey: string | null;
  globalShortcutKey: boolean;
  order: number;
}

export interface MultiItemEntry {
  name: string;
  target: string;
  params?: string | null;
  runAsAdmin: boolean;
  delayMs: number;
}

export interface ItemData {
  startLocation?: string | null;
  target?: string | null;
  params?: string | null;
  runAsAdmin?: boolean;
  icon?: string | null;
  htmlIcon?: string | null;
  remark?: string | null;
  iconBackgroundColor?: boolean;
  iconBackgroundColorValue?: string | null;
  fixedIcon?: boolean;
  multiItemsTimeInterval?: number;
  multiItems?: MultiItemEntry[];
  exists?: boolean | null; // 最近一次检测是否存在（持久化，null=未检测）
  checkedAt?: number | null; // 最近一次检测时间（unix 秒）
  openNumber?: number; // 启动次数
  lastOpen?: number; // 最近一次启动时间（unix 毫秒）
}

export type { AppSettings as LauncherSetting, WebSearchSource } from "../../contracts/appSettings";

export interface ItemCheckResult {
  itemId: number;
  exists: boolean;
  name: string; // 项目名称
  icon?: string | null; // 网页类检测后自动更新的图标 (base64)，未变更为 null/undefined
  title?: string | null; // 网页类检测后自动更新的标题
}

export interface CheckProgress {
  done: number;
  total: number;
}

export interface ScannedProgram {
  name: string;
  target: string;
  params?: string | null;
  icon?: string | null;
  category?: string | null;
  isDir: boolean;
}

export interface AppxItem {
  displayName: string;
  familyName: string;
  appId: string;
  logo?: string | null;
  installedPath?: string | null;
}

export interface ShortcutInfo {
  name: string;
  targetPath: string;
  arguments: string;
  workingDir: string;
  iconPath?: string | null;
  isDir: boolean;
  iconBase64?: string | null;
}

/** 文件索引搜索结果（Everything 式全盘搜索） */
export interface FileHit {
  drive: string;
  path: string;
  name: string;
  isDir: boolean;
  size: number;
  /** 修改时间（unix 秒） */
  modified: number;
}

/** 可索引驱动器状态 */
export interface DriveIndexInfo {
  letter: string;
  root: string;
  /** fixed / removable / network / cdrom / ramdisk / unknown */
  driveType: string;
  totalBytes: number;
  freeBytes: number;
  /** idle / building / ready */
  status: string;
  itemCount: number;
  indexedAt: number;
  elapsedMs: number;
  /** 建立索引时是否遵循 .gitignore */
  respectGitignore: boolean;
}

export interface UrlMetadata {
  title: string;
  icon?: string | null;
  url: string;
}
