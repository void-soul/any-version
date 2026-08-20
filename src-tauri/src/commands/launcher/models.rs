use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 分类数据模型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Classification {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    /// 0: 普通分类, 1: 关联文件夹, 2: 聚合分类
    pub classification_type: i32,
    pub data: ClassificationData,
    pub shortcut_key: Option<String>,
    pub global_shortcut_key: bool,
    pub order: i32,
    #[serde(default)]
    pub child_list: Option<Vec<Classification>>,
    #[serde(default)]
    pub item_count: Option<usize>,
}

/// 分类附加数据
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationData {
    pub icon: Option<String>,
    pub associate_folder_path: Option<String>,
    pub associate_folder_hidden_items: Option<String>,
    #[serde(default = "default_layout")]
    pub item_layout: String,
    #[serde(default = "default_sort")]
    pub item_sort: String,
    pub item_column_number: Option<i32>,
    pub item_icon_size: Option<i32>,
    #[serde(default = "default_show_only")]
    pub item_show_only: String,
    #[serde(default)]
    pub fixed: bool,
    #[serde(default)]
    pub exclude_search: bool,
}

fn default_layout() -> String { "default".to_string() }
fn default_sort() -> String { "default".to_string() }
fn default_show_only() -> String { "default".to_string() }

/// 启动项数据模型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: i64,
    pub classification_id: i64,
    pub name: String,
    /// 类型: 0:文件/应用, 1:文件夹, 2:网址, 3:系统, 4:Appx, 5:多项目
    pub item_type: i32,
    pub data: ItemData,
    pub shortcut_key: Option<String>,
    pub global_shortcut_key: bool,
    pub order: i32,
}

/// 多项目子条目
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MultiItemEntry {
    pub name: String,
    pub target: String,
    pub params: Option<String>,
    pub run_as_admin: bool,
    pub delay_ms: u64,
}

/// 启动项附加数据
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ItemData {
    pub start_location: Option<String>,
    pub target: Option<String>,
    pub params: Option<String>,
    #[serde(default)]
    pub run_as_admin: bool,
    pub icon: Option<String>,
    pub html_icon: Option<String>,
    pub remark: Option<String>,
    #[serde(default)]
    pub icon_background_color: bool,
    #[serde(default)]
    pub fixed_icon: bool,
    #[serde(default)]
    pub multi_items_time_interval: i64,
    pub multi_items: Option<Vec<MultiItemEntry>>,
    /// 最近一次检测是否存在（持久化，None=未检测）
    #[serde(default)]
    pub exists: Option<bool>,
    /// 最近一次检测时间（unix 秒），None=未检测
    #[serde(default)]
    pub checked_at: Option<i64>,
}

/// 启动项检测结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemCheckResult {
    pub item_id: i64,
    pub exists: bool,
    /// 项目名称（供前端实时显示检测到哪一项）
    pub name: String,
    /// 网页类项目检测后自动更新的图标（base64），未变更为 None
    pub icon: Option<String>,
    /// 网页类项目检测后自动更新的标题，未变更为 None
    pub title: Option<String>,
}

/// 检测进度事件载荷（每检测完一项 emit 一次，实时呈现结果）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckProgress {
    pub done: usize,
    pub total: usize,
    /// 当前刚检测完的项目 id（实时高亮结果）
    pub item_id: i64,
    /// 当前刚检测完的项目名称（显示正在检测什么）
    pub name: String,
    /// 该项目是否存在
    pub exists: bool,
    /// 网页类项目更新后的图标（base64），未变更为 None
    pub icon: Option<String>,
    /// 网页类项目更新后的标题，未变更为 None
    pub title: Option<String>,
    /// 是否因用户停止而中止
    pub stopped: bool,
}

/// 启动器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherSetting {
    /// 遗留字段：旧版本用于激活「启动」模块的主热键。
    /// 现已统一到 `module_hotkeys`（所有顶级模块快捷键平等，含「启动」= "launcher"）。
    /// 读取时由 `db::get_settings` 自动迁移进 `module_hotkeys["launcher"]` 并清空本字段。
    #[serde(default)]
    pub show_hide_shortcut_key: String,
    /// 各顶级模块的独立唤起快捷键：module_id -> 热键字符串（如 "F2"、"Win+V"）。
    /// 所有模块（含「启动」= "launcher"）地位平等，三段式切换：
    /// 程序隐藏则唤起并切到该模块；已激活则切换到该模块；正处该模块则隐藏。
    #[serde(default = "default_module_hotkeys")]
    pub module_hotkeys: HashMap<String, String>,
    // ---- 视图设置（全局，应用到所有分类）----
    /// 项目图标大小（px），默认 32
    #[serde(default = "default_item_icon_size")]
    pub item_icon_size: i32,
    /// 网格列数（0 = 自适应），默认 0
    #[serde(default)]
    pub item_column_number: i32,
    /// 卡片密度：compact / cozy / spacious，默认 cozy
    #[serde(default = "default_card_density")]
    pub card_density: String,
    /// 是否显示项目名称，默认 true
    #[serde(default = "default_show_item_name")]
    pub show_item_name: bool,
    /// 是否显示图标背景色块，默认 false
    #[serde(default)]
    pub icon_background_color: bool,
    /// 项目文字大小（px），默认 12
    #[serde(default = "default_item_font_size")]
    pub item_font_size: i32,
    /// 项目卡片圆角（px），默认 12
    #[serde(default = "default_item_radius")]
    pub item_radius: i32,
    /// 是否显示项目卡片边框，默认 true
    #[serde(default = "default_item_border")]
    pub item_border: bool,
    /// 分类文字大小（px），默认 12
    #[serde(default = "default_category_font_size")]
    pub category_font_size: i32,
    /// 分类（分组）之间的垂直间距（px），默认 24
    #[serde(default = "default_category_gap")]
    pub category_gap: i32,
}

fn default_module_hotkeys() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("launcher".to_string(), "Alt+Space".to_string());
    m
}
fn default_item_icon_size() -> i32 {
    32
}
fn default_card_density() -> String {
    "cozy".to_string()
}
fn default_show_item_name() -> bool {
    true
}
fn default_item_font_size() -> i32 {
    12
}
fn default_item_radius() -> i32 {
    12
}
fn default_item_border() -> bool {
    true
}
fn default_category_font_size() -> i32 {
    12
}
fn default_category_gap() -> i32 {
    24
}

impl Default for LauncherSetting {
    fn default() -> Self {
        Self {
            show_hide_shortcut_key: String::new(),
            module_hotkeys: default_module_hotkeys(),
            item_icon_size: default_item_icon_size(),
            item_column_number: 0,
            card_density: default_card_density(),
            show_item_name: default_show_item_name(),
            icon_background_color: false,
            item_font_size: default_item_font_size(),
            item_radius: default_item_radius(),
            item_border: default_item_border(),
            category_font_size: default_category_font_size(),
            category_gap: default_category_gap(),
        }
    }
}

/// 网页搜索源
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchSource {
    pub id: String,
    pub keyword: String,
    pub name: String,
    pub url: String,
    pub description: Option<String>,
}

pub fn default_web_search_sources() -> Vec<WebSearchSource> {
    vec![
        WebSearchSource {
            id: "1".to_string(),
            keyword: "bd".to_string(),
            name: "百度".to_string(),
            url: "https://www.baidu.com/s?wd=%s".to_string(),
            description: Some("百度搜索".to_string()),
        },
        WebSearchSource {
            id: "2".to_string(),
            keyword: "gg".to_string(),
            name: "Google".to_string(),
            url: "https://www.google.com/search?q=%s".to_string(),
            description: Some("Google 搜索".to_string()),
        },
        WebSearchSource {
            id: "3".to_string(),
            keyword: "gh".to_string(),
            name: "GitHub".to_string(),
            url: "https://github.com/search?q=%s".to_string(),
            description: Some("GitHub 代码与仓库搜索".to_string()),
        },
        WebSearchSource {
            id: "4".to_string(),
            keyword: "bing".to_string(),
            name: "必应".to_string(),
            url: "https://cn.bing.com/search?q=%s".to_string(),
            description: Some("微软必应搜索".to_string()),
        },
        WebSearchSource {
            id: "5".to_string(),
            keyword: "bili".to_string(),
            name: "哔哩哔哩".to_string(),
            url: "https://search.bilibili.com/all?keyword=%s".to_string(),
            description: Some("B站视频搜索".to_string()),
        },
    ]
}

/// 扫描程序/快捷方式条目
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedProgram {
    pub name: String,
    pub target: String,
    pub params: Option<String>,
    pub icon: Option<String>,
    pub category: Option<String>,
    pub is_dir: bool,
}

/// 扫描到的 Appx / UWP 应用
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppxItem {
    pub display_name: String,
    pub family_name: String,
    pub app_id: String,
    pub logo: Option<String>,
    pub installed_path: Option<String>,
}

/// 抓取网页元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlMetadata {
    pub title: String,
    pub icon: Option<String>,
    pub url: String,
}

/// 快捷方式解析信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutInfo {
    pub name: String,
    pub target_path: String,
    pub arguments: String,
    pub working_dir: String,
    pub icon_path: Option<String>,
    pub is_dir: bool,
    pub icon_base64: Option<String>,
}

/// 浏览器书签导入结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserImportResult {
    pub count: usize,
    pub category_id: i64,
}
