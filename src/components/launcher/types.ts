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
  itemSort?: "default" | "initial";
  itemColumnNumber?: number | null;
  itemIconSize?: number | null;
  itemShowOnly?: "default" | "file" | "folder";
  fixed?: boolean;
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
  multiItemsTimeInterval?: number;
  multiItems?: MultiItemEntry[];
  exists?: boolean | null; // 最近一次检测是否存在（持久化，null=未检测）
  checkedAt?: number | null; // 最近一次检测时间（unix 秒）
}

export interface LauncherSetting {
  showHideShortcutKey: string; // 唤起/隐藏主程序界面全局快捷键 (默认 Alt+Space)
  // ---- 视图设置（全局，应用到所有分类）----
  itemIconSize?: number; // 项目图标大小 (px)，默认 32
  itemColumnNumber?: number; // 网格列数 (0=自适应)，默认 0
  cardDensity?: "compact" | "cozy" | "spacious"; // 卡片密度，默认 cozy
  showItemName?: boolean; // 是否显示项目名称，默认 true
  iconBackgroundColor?: boolean; // 是否显示图标背景色块，默认 false
  itemFontSize?: number; // 项目文字大小 (px)，默认 12
  itemRadius?: number; // 项目卡片圆角 (px)，默认 12
  itemBorder?: boolean; // 是否显示项目卡片边框，默认 true
  categoryFontSize?: number; // 分类文字大小 (px)，默认 12
  categoryGap?: number; // 分类（分组）之间的垂直间距 (px)，默认 24
}

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

export interface UrlMetadata {
  title: string;
  icon?: string | null;
  url: string;
}
