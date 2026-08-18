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

export interface ClassificationData {
  icon?: string | null;
  associateFolderPath?: string | null;
  associateFolderHiddenItems?: string | null;
  itemLayout?: "default" | "tile" | "list" | "compact" | "icon_only";
  itemSort?: "default" | "initial" | "openNumber" | "lastOpen";
  itemColumnNumber?: number | null;
  itemIconSize?: number | null;
  itemShowOnly?: "default" | "file" | "folder";
  fixed?: boolean;
  aggregateItemCount?: number;
  aggregateSort?: "openNumber" | "lastOpen";
  excludeSearch?: boolean;
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
  fixedIcon?: boolean;
  openNumber?: number;
  lastOpen?: number;
  multiItemsTimeInterval?: number;
  multiItems?: MultiItemEntry[];
}

export interface WebSearchSource {
  id: string;
  keyword: string;
  name: string;
  url: string;
  description?: string | null;
}

export interface LauncherSetting {
  showHideShortcutKey: string;
  openMode: "single_click" | "double_click";
  openAfterHide: boolean;
  itemLayout: "tile" | "list" | "compact" | "icon_only";
  columnCount: number; // 0: 自动自适应, 2..12
  density: "compact" | "standard" | "relaxed";
  iconSize: number; // 24 .. 96
  nameDisplay: "show" | "hide" | "two_lines";
  defaultRunAsAdmin: boolean;
  webSearchSources: WebSearchSource[];
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

export interface UrlMetadata {
  title: string;
  icon?: string | null;
  url: string;
}
