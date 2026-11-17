//! 文件夹（即「分组」）的路径运算与增删改。
//!
//! 设计：连接不维护独立的关联表，文件夹路径直接存在连接的 `group` 字段里
//! （`prod/web` 表示放在根下 `prod/web`）。空文件夹没有连接可依附，因此单独
//! 存一份路径清单（`rdp/groups.json`），只用于让空文件夹在界面上继续存在。
//!
//! 这里的函数都是**纯逻辑**（不碰 IO），便于覆盖前缀边界这类容易写错的地方。

use super::models::RdpConnection;

/// 统一分隔符、去首尾与重复斜杠；`root` 与空串都表示根。
pub fn normalize_path(raw: &str) -> String {
    let replaced = raw.replace('\\', "/");
    let parts: Vec<&str> = replaced
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() == 1 && parts[0].eq_ignore_ascii_case("root") {
        return String::new();
    }
    parts.join("/")
}

/// 把 `path` 从 `from` 子树搬到 `to` 子树下。只承认真正的层级关系。
pub fn rebase_path(path: &str, from: &str, to: &str) -> String {
    let current = normalize_path(path);
    let src = normalize_path(from);
    let dst = normalize_path(to);
    if src.is_empty() {
        return current;
    }
    if current == src {
        return dst;
    }
    let prefix = format!("{src}/");
    if !current.starts_with(&prefix) {
        return current;
    }
    let rest = &current[prefix.len()..];
    if dst.is_empty() {
        rest.to_string()
    } else {
        format!("{dst}/{rest}")
    }
}

fn is_same_or_child(path: &str, base: &str) -> bool {
    if base.is_empty() {
        return false;
    }
    path == base || path.starts_with(&format!("{base}/"))
}

fn push_with_ancestors(records: &mut Vec<String>, path: &str) {
    let normalized = normalize_path(path);
    if normalized.is_empty() {
        return;
    }
    let parts: Vec<&str> = normalized.split('/').collect();
    for i in 1..=parts.len() {
        let ancestor = parts[..i].join("/");
        if !records.iter().any(|r| r == &ancestor) {
            records.push(ancestor);
        }
    }
}

/// 新建文件夹（自动补齐中间层）。返回是否有新增。
pub fn create_folder(records: &mut Vec<String>, path: &str) -> bool {
    let before = records.len();
    push_with_ancestors(records, path);
    records.len() > before
}

/// 重命名 / 移动文件夹：改所有后代文件夹记录与连接路径。
pub fn rename_folder(
    items: &mut [RdpConnection],
    records: &mut Vec<String>,
    from: &str,
    to: &str,
) -> usize {
    let src = normalize_path(from);
    if src.is_empty() {
        return 0;
    }
    let dst = normalize_path(to);
    if src == dst {
        return 0;
    }

    let mut changed = 0;
    for item in items.iter_mut() {
        let current = normalize_path(&item.group);
        if is_same_or_child(&current, &src) {
            item.group = rebase_path(&current, &src, &dst);
            changed += 1;
        }
    }

    let mut next: Vec<String> = Vec::new();
    for record in records.iter() {
        let current = normalize_path(record);
        if is_same_or_child(&current, &src) {
            let moved = rebase_path(&current, &src, &dst);
            if !moved.is_empty() {
                push_with_ancestors(&mut next, &moved);
            }
        } else if !current.is_empty() {
            push_with_ancestors(&mut next, &current);
        }
    }
    if !dst.is_empty() {
        push_with_ancestors(&mut next, &dst);
    }
    *records = next;
    changed
}

/// 删除文件夹：成员上移到父级，文件夹记录（含后代）移除，**连接本身不删**。
pub fn delete_folder(items: &mut [RdpConnection], records: &mut Vec<String>, path: &str) -> usize {
    let target = normalize_path(path);
    if target.is_empty() {
        return 0;
    }
    let parent = target.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();

    let mut changed = 0;
    for item in items.iter_mut() {
        let current = normalize_path(&item.group);
        if is_same_or_child(&current, &target) {
            item.group = parent.clone();
            changed += 1;
        }
    }

    records.retain(|record| !is_same_or_child(&normalize_path(record), &target));
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_root_and_separators() {
        assert_eq!(normalize_path("  a/b "), "a/b");
        assert_eq!(normalize_path("a\\b"), "a/b");
        assert_eq!(normalize_path("ROOT"), "");
    }

    #[test]
    fn rebase_keeps_prefix_boundaries() {
        assert_eq!(rebase_path("ab", "a", "x"), "ab");
        assert_eq!(rebase_path("a/b", "a", "x"), "x/b");
    }
}
