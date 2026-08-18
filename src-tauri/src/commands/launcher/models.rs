use serde::{Deserialize, Serialize};

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
    #[serde(default = "default_aggregate_count")]
    pub aggregate_item_count: i32,
    #[serde(default = "default_aggregate_sort")]
    pub aggregate_sort: String,
    #[serde(default)]
    pub exclude_search: bool,
}

fn default_layout() -> String { "default".to_string() }
fn default_sort() -> String { "default".to_string() }
fn default_show_only() -> String { "default".to_string() }
fn default_aggregate_count() -> i32 { 30 }
fn default_aggregate_sort() -> String { "openNumber".to_string() }

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
    pub open_number: i64,
    #[serde(default)]
    pub last_open: i64,
    #[serde(default)]
    pub multi_items_time_interval: i64,
    pub multi_items: Option<Vec<MultiItemEntry>>,
}

/// 启动器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherSetting {
    #[serde(default = "default_hotkey")]
    pub show_hide_shortcut_key: String,
    #[serde(default = "default_open_mode")]
    pub open_mode: String,
    #[serde(default)]
    pub open_after_hide: bool,
    #[serde(default = "default_item_layout")]
    pub item_layout: String,
    #[serde(default = "default_column_count")]
    pub column_count: i32,
    #[serde(default = "default_density")]
    pub density: String,
    #[serde(default = "default_icon_size")]
    pub icon_size: i32,
    #[serde(default = "default_name_display")]
    pub name_display: String,
    #[serde(default)]
    pub default_run_as_admin: bool,
    #[serde(default = "default_web_search_sources")]
    pub web_search_sources: Vec<WebSearchSource>,
}

fn default_hotkey() -> String { "Alt+Space".to_string() }
fn default_open_mode() -> String { "single_click".to_string() }
fn default_item_layout() -> String { "tile".to_string() }
fn default_column_count() -> i32 { 0 }
fn default_density() -> String { "standard".to_string() }
fn default_icon_size() -> i32 { 48 }
fn default_name_display() -> String { "show".to_string() }

impl Default for LauncherSetting {
    fn default() -> Self {
        Self {
            show_hide_shortcut_key: default_hotkey(),
            open_mode: default_open_mode(),
            open_after_hide: false,
            item_layout: default_item_layout(),
            column_count: default_column_count(),
            density: default_density(),
            icon_size: default_icon_size(),
            name_display: default_name_display(),
            default_run_as_admin: false,
            web_search_sources: default_web_search_sources(),
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
