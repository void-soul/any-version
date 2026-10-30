use super::*;

/// 列出收藏条目。
///
/// `sort`：已收敛为「按收藏时间」（历史 `created` / `updated` 选项已移除）。
/// `favorited_since`：只保留收藏时间不早于该时刻的条目（本地时间字符串，由前端按
/// 「今天 / 近 7 天 / 近 30 天 / 今年」这类预设算好再传，后端不猜时区）。
#[tauri::command]
pub fn fav_list(
    source: Option<String>,
    category_id: Option<i64>,
    status: Option<String>,
    keyword: Option<String>,
    sort: Option<String>,
    favorited_since: Option<String>,
    starred_only: Option<bool>,
    limit: Option<usize>,
) -> Result<Vec<db::FavoriteRow>, String> {
    db::with_conn(|conn| {
        db::list(
            conn,
            &db::ListFilter {
                source,
                category_id,
                status,
                keyword,
                sort,
                favorited_since,
                starred_only: starred_only.unwrap_or(false),
                limit: limit.unwrap_or(0),
            },
        )
    })
}

/// 星标（特别关注）开关。
///
/// 为什么不用「小型号」实现：这是跨设备的个人标记，不是要还原平台的任何状态 ——
/// 平台侧没有这个概念，拉下来再同步只会造成两边不一致。
#[tauri::command]
pub fn fav_set_starred(id: i64, starred: bool) -> Result<(), String> {
    db::with_conn(|conn| db::set_starred(conn, id, starred))
}

/// 导入浏览器收藏夹（Edge / Chrome）。
///
/// 从启动模块搬过来的能力，但**书签目录会建成多级分类树**（以前压平成一层的做法
/// 把用户整理好的目录结构丢了）。可以多设备/多 Profile 反复导入：靠
/// `(source, external_id)` 去重，第二次只是把条目挂到新出现的目录上。
#[tauri::command]
pub fn fav_import_bookmarks(
    browser: String,
    custom_path: Option<String>,
) -> Result<crate::commands::favorites::bookmarks::BookmarkImportResult, String> {
    crate::commands::favorites::bookmarks::import(&browser, custom_path.as_deref())
}

/// 列出整棵分类树（含条目数）。
#[tauri::command]
pub fn fav_list_categories() -> Result<Vec<db::CategoryNode>, String> {
    db::with_conn(|conn| db::list_category_tree(conn))
}

/// 新建分类（顶层传 null）。
///
/// `kind`：`topic` = 主题分类（AI 归类会往里写）· `follow` = 关注分组（人工维护，
/// AI 归类不碰）。两者必须分开，否则用户整理的「待读 / 重点」会被重新归类清空。
#[tauri::command]
pub fn fav_create_category(
    name: String,
    parent_id: Option<i64>,
    kind: Option<String>,
) -> Result<i64, String> {
    let k = match kind.as_deref() {
        Some("follow") => "follow",
        _ => "topic",
    };
    db::with_conn(|conn| db::create_category(conn, &name, parent_id, k))
}

/// 重命名分类。
#[tauri::command]
pub fn fav_rename_category(id: i64, name: String) -> Result<(), String> {
    db::with_conn(|conn| db::rename_category(conn, id, &name))
}

/// 移动分类（换父级；顶层传 null）。自己 / 子孙作目标会被拒绝。
#[tauri::command]
pub fn fav_move_category(id: i64, parent_id: Option<i64>) -> Result<(), String> {
    db::with_conn(|conn| db::move_category(conn, id, parent_id))
}

/// 删除分类及其所有子分类（**不动条目本身**，只解开关联）。
#[tauri::command]
pub fn fav_delete_category(id: i64) -> Result<usize, String> {
    db::with_conn(|conn| db::delete_category(conn, id))
}

/// 同级排序：`[(id, sort_order)]` 批量写回。
#[tauri::command]
pub fn fav_reorder_categories(orders: Vec<(i64, i32)>) -> Result<(), String> {
    db::with_conn(|conn| db::reorder_categories(conn, &orders))
}

/// 全量替换某条目的分类（人工操作 → 同时锁定，AI 归类不再改动它）。
#[tauri::command]
pub fn fav_set_item_categories(id: i64, category_ids: Vec<i64>) -> Result<(), String> {
    db::with_conn(|conn| db::set_item_categories_manual(conn, id, &category_ids))
}

/// 人工设置标签（全量替换 + 锁定，后续 AI 归类不再改动）。
#[tauri::command]
pub fn fav_set_tags(id: i64, tags: Vec<String>) -> Result<(), String> {
    db::with_conn(|conn| db::set_tags(conn, id, &tags))
}

/// 删除本地条目（**只删本地**，不动平台）。
///
/// 同时立一条**删除墓碑**（`favorite_deleted`）：平台上的收藏还在，下次导入一定会再拉到它，
/// 没有碑就会被当成新条目重新插入（表现为「删了又回来」）。
#[tauri::command]
pub fn fav_delete(id: i64) -> Result<bool, String> {
    db::with_conn(|conn| db::delete(conn, id))
}

/// 列出删除墓碑（设置页展示「已删除 n 条」并允许重新纳入）。
#[tauri::command]
pub fn fav_list_deleted() -> Result<Vec<db::FavoriteDeletedRow>, String> {
    db::with_conn(|conn| db::list_deleted(conn))
}

/// 清除删除墓碑 → 这些条目下次导入会重新进来。
///
/// `source` + `externalId` 都给 = 只恢复一条；只给 source = 恢复该来源全部；都不给 = 全部恢复。
#[tauri::command]
pub fn fav_restore_deleted(
    source: Option<String>,
    external_id: Option<String>,
) -> Result<usize, String> {
    db::with_conn(|conn| db::clear_deleted(conn, source.as_deref(), external_id.as_deref()))
}

/// 概览计数。
#[tauri::command]
pub fn fav_stats() -> Result<db::FavoriteStats, String> {
    db::with_conn(|conn| db::stats(conn))
}
