//! 轻量 `.gitignore` 匹配器（零第三方依赖）。
//!
//! 只覆盖索引扫描需要的常用语义：
//! - 每层目录的 `.gitignore` 叠加生效，深层规则覆盖浅层；
//! - `!` 否定规则；行尾 `/` 表示仅目录；
//! - 含 `/` 的模式相对 `.gitignore` 所在目录锚定，否则匹配任意层级的同名段；
//! - `*` 不跨目录、`**` 跨目录、`?` 单字符、`[abc]` / `[a-z]` / `[!abc]` 字符组；
//! - 大小写不敏感（贴合 Windows 文件系统与 `core.ignorecase=true` 的默认行为）。
//!
//! 不支持的边角语义：`\ ` 转义空格、`core.excludesFile` 全局忽略、
//! 被忽略目录内再用 `!` 重新包含（与 git 一致：父目录被忽略后无法再包含）。

use std::path::Path;

#[derive(Debug, Clone)]
struct Rule {
    pattern: String,
    negated: bool,
    dir_only: bool,
    /// 含 `/` 的模式相对 `.gitignore` 所在目录锚定。
    anchored: bool,
}

#[derive(Debug, Clone)]
pub struct IgnoreFile {
    /// 相对盘根的目录（`""` 表示盘根），统一用 `/`、结尾不带 `/`。
    base: String,
    rules: Vec<Rule>,
}

impl IgnoreFile {
    /// 目录里没有 `.gitignore`（或读不到）时返回 None。
    pub fn load(dir: &Path, base: &str) -> Option<Self> {
        let text = std::fs::read_to_string(dir.join(".gitignore")).ok()?;
        let rules = parse_rules(&text);
        if rules.is_empty() {
            return None;
        }
        Some(Self {
            base: base.to_string(),
            rules,
        })
    }

    /// `Some(true)` = 忽略，`Some(false)` = 被否定规则重新包含，`None` = 未命中。
    fn decide(&self, rel: &str, is_dir: bool) -> Option<bool> {
        let sub = if self.base.is_empty() {
            rel
        } else {
            rel.strip_prefix(self.base.as_str())
                .and_then(|rest| rest.strip_prefix('/'))?
        };
        let mut result = None;
        for rule in &self.rules {
            if rule.dir_only && !is_dir {
                continue;
            }
            if rule.matches(sub) {
                result = Some(!rule.negated);
            }
        }
        result
    }
}

impl Rule {
    fn matches(&self, sub: &str) -> bool {
        if self.anchored {
            glob_match(&self.pattern, sub)
        } else {
            sub.split('/').any(|seg| glob_match(&self.pattern, seg))
        }
    }
}

/// 按遍历深度维护的 `.gitignore` 规则栈。
///
/// 扫描是前序遍历：进入目录时压入该目录的规则，离开（遇到同层或更浅的条目）时弹出，
/// 这样每个条目只需要和当前路径上的规则比较。
#[derive(Debug, Default)]
pub struct IgnoreStack {
    /// (装载时的目录深度, 规则文件)
    files: Vec<(usize, IgnoreFile)>,
}

impl IgnoreStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// 弹出深度 >= `depth` 的规则；随后若 `dir` 下有 `.gitignore` 则压入。
    pub fn enter(&mut self, depth: usize, dir: &Path, rel: &str) {
        self.leave_to(depth);
        if let Some(file) = IgnoreFile::load(dir, rel) {
            self.files.push((depth, file));
        }
    }

    /// 离开当前目录层：丢弃所有在 `depth` 及更深层装载的规则。
    pub fn leave_to(&mut self, depth: usize) {
        while self.files.last().is_some_and(|(d, _)| *d >= depth) {
            self.files.pop();
        }
    }

    /// 浅层规则先判、深层覆盖，最后一次命中的规则生效（与 git 一致）。
    pub fn is_ignored(&self, rel: &str, is_dir: bool) -> bool {
        let mut result = None;
        for (_, file) in &self.files {
            if let Some(decision) = file.decide(rel, is_dir) {
                result = Some(decision);
            }
        }
        result.unwrap_or(false)
    }
}

fn parse_rules(text: &str) -> Vec<Rule> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let mut line = raw.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut negated = false;
        if let Some(rest) = line.strip_prefix("\\!") {
            line = rest; // 转义的 `!` 按字面量处理
        } else if let Some(rest) = line.strip_prefix("\\#") {
            line = rest;
        } else if let Some(rest) = line.strip_prefix('!') {
            negated = true;
            line = rest;
        }
        let body = line.trim_start();
        let dir_only = body.ends_with('/');
        let body = body.trim_end_matches('/');
        let pattern = body.trim_start_matches('/');
        if pattern.is_empty() {
            continue;
        }
        out.push(Rule {
            // 统一小写：调用方传入的相对路径同样是小写，从而实现大小写不敏感匹配
            pattern: pattern.to_lowercase(),
            negated,
            dir_only,
            anchored: body.contains('/'),
        });
    }
    out
}

/// 通配匹配：`*` 不跨 `/`，`**` 跨 `/`，`?` 单字符，`[]` 字符组。
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    match_from(&p, &t)
}

fn match_from(p: &[char], t: &[char]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }
    match p[0] {
        '*' => {
            if p.len() > 1 && p[1] == '*' {
                let mut rest = &p[2..];
                if rest.first() == Some(&'/') {
                    // `**/` 允许匹配零层目录
                    if match_from(&rest[1..], t) {
                        return true;
                    }
                    rest = &rest[1..];
                }
                for i in 0..=t.len() {
                    if match_from(rest, &t[i..]) {
                        return true;
                    }
                }
                false
            } else {
                let rest = &p[1..];
                if match_from(rest, t) {
                    return true;
                }
                for i in 0..t.len() {
                    if t[i] == '/' {
                        break;
                    }
                    if match_from(rest, &t[i + 1..]) {
                        return true;
                    }
                }
                false
            }
        }
        '?' => !t.is_empty() && t[0] != '/' && match_from(&p[1..], &t[1..]),
        '[' => match_class(p, t),
        '\\' if p.len() > 1 => !t.is_empty() && t[0] == p[1] && match_from(&p[2..], &t[1..]),
        c => !t.is_empty() && t[0] == c && match_from(&p[1..], &t[1..]),
    }
}

fn match_class(p: &[char], t: &[char]) -> bool {
    if t.is_empty() || t[0] == '/' {
        return false;
    }
    let mut i = 1;
    let negated = matches!(p.get(i), Some('!') | Some('^'));
    if negated {
        i += 1;
    }
    let mut hit = false;
    while i < p.len() && p[i] != ']' {
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            if p[i] <= t[0] && t[0] <= p[i + 2] {
                hit = true;
            }
            i += 3;
        } else {
            if p[i] == t[0] {
                hit = true;
            }
            i += 1;
        }
    }
    if i >= p.len() {
        // 没有闭合的 `]`，按字面量处理
        return t[0] == '[' && match_from(&p[1..], &t[1..]);
    }
    if hit == negated {
        return false;
    }
    match_from(&p[i + 1..], &t[1..])
}

#[cfg(test)]
mod tests {
    use super::{glob_match, IgnoreFile, IgnoreStack};
    use std::path::Path;

    fn rules(text: &str, base: &str) -> IgnoreFile {
        IgnoreFile {
            base: base.to_string(),
            rules: super::parse_rules(text),
        }
    }

    #[test]
    fn glob_basics() {
        assert!(glob_match("*.log", "a.log"));
        assert!(!glob_match("*.log", "a/b.log"));
        assert!(glob_match("**/*.log", "a/b/c.log"));
        assert!(glob_match("**/node_modules", "x/node_modules"));
        assert!(glob_match("target", "target"));
        assert!(glob_match("a?c", "abc"));
        assert!(glob_match("a[0-9]c", "a7c"));
        assert!(!glob_match("a[!0-9]c", "a7c"));
        assert!(glob_match("a[!0-9]c", "abc"));
    }

    #[test]
    fn rules_match_any_depth_when_unanchored() {
        let f = rules("node_modules/\n*.log\n", "");
        assert_eq!(f.decide("proj/node_modules", true), Some(true));
        // 目录限定规则不作用于同名文件
        assert_eq!(f.decide("proj/node_modules", false), None);
        assert_eq!(f.decide("a/b/c.log", false), Some(true));
        assert_eq!(f.decide("a/b/c.txt", false), None);
    }

    #[test]
    fn anchored_rules_only_match_from_base() {
        let f = rules("/dist\nsrc/*.tmp\n", "proj");
        assert_eq!(f.decide("proj/dist", true), Some(true));
        assert_eq!(f.decide("proj/sub/dist", true), None);
        assert_eq!(f.decide("proj/src/a.tmp", false), Some(true));
        assert_eq!(f.decide("proj/src/deep/a.tmp", false), None);
        // 不在 base 之下的路径不参与
        assert_eq!(f.decide("other/dist", true), None);
    }

    #[test]
    fn negation_reincludes() {
        let f = rules("*.log\n!keep.log\n", "");
        assert_eq!(f.decide("a/x.log", false), Some(true));
        assert_eq!(f.decide("a/keep.log", false), Some(false));
    }

    #[test]
    fn matching_is_case_insensitive() {
        let f = rules("Node_Modules/\n*.LOG\n", "");
        assert_eq!(f.decide("proj/node_modules", true), Some(true));
        assert_eq!(f.decide("a/x.log", false), Some(true));
    }

    #[test]
    fn nested_files_override_parent() {
        let mut stack = IgnoreStack::new();
        // 用不存在的目录，避免读到真实磁盘上的 .gitignore
        stack.enter(0, Path::new("__no_such_dir__"), "");
        stack.push_for_test(1, "*.log\n", "repo");
        assert!(stack.is_ignored("repo/a.log", false));
        stack.push_for_test(2, "!a.log\n", "repo/debug");
        assert!(!stack.is_ignored("repo/debug/a.log", false));
        assert!(stack.is_ignored("repo/other/a.log", false));
        // 离开 repo/debug 后，否定规则不再生效
        stack.leave_to(2);
        assert!(stack.is_ignored("repo/x.log", false));
    }

    impl IgnoreStack {
        /// 测试用：直接注入规则，避免依赖真实文件系统。
        fn push_for_test(&mut self, depth: usize, text: &str, base: &str) {
            self.files.push((depth, rules(text, base)));
        }
    }
}
