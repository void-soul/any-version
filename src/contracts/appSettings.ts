// Application-wide preferences. Storage is an implementation detail of the backend.
export interface AppSettings {
  showHideShortcutKey?: string; // 遗留字段（旧版「启动」模块主热键），已迁移到 moduleHotkeys.launcher
  moduleHotkeys?: Record<string, string>; // 各顶级模块快捷键：moduleId -> 热键字符串（含「启动」= "launcher"）
  selectionTranslateHotkey?: string; // 独立「划词翻译」热键：任意程序选中文本后按下 → 悬浮翻译
  mindmapQuickHotkey?: string; // 独立「思维导图节点速记」热键：呼出节点悬浮窗随手记录，默认 Shift+F3
  mindmapStickerHotkey?: string; // 独立「思维导图贴纸」热键：呼出贴纸悬浮窗，默认 Shift+F4
  fileSearchHotkey?: string; // 独立「文件搜索」热键：呼出 Everything 式文件搜索悬浮窗，默认 Ctrl+Alt+Space
  musicPlayPauseHotkey?: string; // 音乐播放器：播放/暂停（全局热键，托盘状态同样生效）
  musicPrevHotkey?: string; // 音乐播放器：上一首
  musicNextHotkey?: string; // 音乐播放器：下一首
  // ---- 视图设置（全局，应用到所有分类）----
  itemIconSize?: number; // 项目图标大小 (px)，默认 32
  itemColumnNumber?: number; // 网格列数 (0=自适应)，默认 0
  cardDensity?: "compact" | "cozy" | "spacious"; // 卡片密度，默认 cozy
  showItemName?: boolean; // 是否显示项目名称，默认 true
  iconBackgroundColor?: boolean; // 是否显示图标背景色块，默认 false
  showOnlyValid?: boolean; // 只显示有效项目（检测存在 或 未检测），默认 false 显示全部
  itemFontSize?: number; // 项目文字大小 (px)，默认 12
  itemRadius?: number; // 项目卡片圆角 (px)，默认 12
  itemBorder?: boolean; // 是否显示项目卡片边框，默认 true
  categoryFontSize?: number; // 分类文字大小 (px)，默认 12
  categoryGap?: number; // 分类（分组）之间的垂直间距 (px)，默认 24
  externalEditor?: string; // 全局外部编辑器（路径或 PATH 命令，如 "code"；支持 {file} 占位符；空 = 未配置，回退资源管理器定位）
  webSearchSources?: WebSearchSource[]; // 快捷搜索的网页搜索引擎（输入「bd 关键词」直接搜索）
}

/** 网页搜索源 */
export interface WebSearchSource {
  id: string;
  keyword: string;
  name: string;
  url: string; // 用 %s 占位关键词
  description?: string | null;
}


export type AppSettingsPatch = Omit<Partial<AppSettings>, "moduleHotkeys"> & { moduleHotkeys?: Record<string, string | null> };
