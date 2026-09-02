use std::path::Path;

/// A minimal .gitignore-style matcher supporting the common patterns:
/// - blank lines and `#` comments are ignored
/// - negation with leading `!`
/// - trailing `/` = directory-only pattern
/// - leading `/` = root-anchored pattern
/// - `*` (any run of chars except `/`) and `?` (single char) wildcards
/// - `**` between slashes matches zero or more directories
/// - a pattern without a slash (after normalization) matches at any depth
#[derive(Debug, Default)]
pub struct IgnoreRules {
    /// (pattern segments, dir_only, negated, root_anchored)
    rules: Vec<Rule>,
}

#[derive(Debug)]
struct Rule {
    segments: Vec<String>,
    dir_only: bool,
    negated: bool,
    root_anchored: bool,
}

impl IgnoreRules {
    /// Build rules from a project root by reading `.gitignore` and
    /// `.claudeignore` (plus `.git/info/exclude`) if present. Missing files
    /// are fine; the matcher then simply matches nothing.
    pub fn load(root: &Path) -> IgnoreRules {
        let mut rules = IgnoreRules::default();
        for name in [".gitignore", ".claudeignore"] {
            let path = root.join(name);
            if let Ok(data) = std::fs::read_to_string(&path) {
                rules.add_lines(&data);
            }
        }
        // Global git exclude, when the project is a git work tree.
        let exclude = root.join(".git/info/exclude");
        if let Ok(data) = std::fs::read_to_string(&exclude) {
            rules.add_lines(&data);
        }
        rules
    }

    /// Parse pattern lines (used by tests too).
    pub fn add_lines(&mut self, text: &str) {
        for raw in text.lines() {
            let line = raw.trim_end();
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let mut pattern = line.trim().to_string();
            let mut negated = false;
            if let Some(rest) = pattern.strip_prefix('!') {
                negated = true;
                pattern = rest.trim().to_string();
            }
            if pattern.is_empty() {
                continue;
            }
            // Trailing spaces are insignificant unless escaped; keep it simple.
            let pattern = pattern.trim_end().to_string();
            let dir_only = pattern.ends_with('/');
            let pattern = pattern.trim_end_matches('/').to_string();
            if pattern.is_empty() {
                continue;
            }
            let root_anchored = pattern.starts_with('/');
            let pattern = pattern.trim_start_matches('/').to_string();
            let segments: Vec<String> = pattern
                .split('/')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            if segments.is_empty() {
                continue;
            }
            self.rules.push(Rule {
                segments,
                dir_only,
                negated,
                root_anchored,
            });
        }
    }

    /// True when `rel` (project-relative, `/`-separated, no leading slash)
    /// should be skipped. `is_dir` matters for directory-only patterns.
    /// Later rules win over earlier ones (gitignore negation semantics).
    pub fn is_ignored(&self, rel: &str, is_dir: bool) -> bool {
        let segments: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return false;
        }
        let mut ignored = false;
        for rule in &self.rules {
            if rule_matches(rule, &segments, is_dir) {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}

fn rule_matches(rule: &Rule, segments: &[&str], is_dir: bool) -> bool {
    if rule.dir_only && !is_dir {
        // A directory-only pattern can still ignore files *inside* the
        // matching directory, so the path must contain the pattern as a
        // leading portion; handled by the matching loop below via is_dir
        // on the prefix. Simplest correct approach: files only match a
        // dir_only rule if some ancestor directory matches the pattern.
    }
    // Try to anchor the pattern at every allowed starting position.
    let start_max = if rule.root_anchored { 1 } else { segments.len() };
    for start in 0..start_max {
        if match_segments(&rule.segments, &segments[start..], is_dir, rule.dir_only) {
            return true;
        }
    }
    false
}

fn match_segments(
    pat: &[String],
    seg: &[&str],
    seg_is_dir: bool,
    dir_only: bool,
) -> bool {
    if pat.is_empty() {
        return false;
    }
    // `**` as the last pattern segment consumes the rest.
    if pat.len() == 1 && pat[0] == "**" {
        return true;
    }
    let head = &pat[0];
    if head == "**" {
        // `**/rest` — try consuming zero or more segments.
        for skip in 0..=seg.len() {
            if match_segments(&pat[1..], &seg[skip..], seg_is_dir, dir_only) {
                return true;
            }
        }
        return false;
    }
    if seg.is_empty() {
        return false;
    }
    if !glob_match(head, seg[0]) {
        return false;
    }
    if pat.len() == 1 {
        // Full match. Directory-only patterns must match a directory
        // segment; a file matches only when an ancestor matched — but in
        // this position the matched segment is the file itself, so a
        // dir_only rule does not apply to it.
        if dir_only && !seg_is_dir && seg.len() == 1 {
            return false;
        }
        return true;
    }
    match_segments(&pat[1..], &seg[1..], seg_is_dir, dir_only)
}

/// Single-segment glob: `*` = any run without `/`, `?` = one char.
/// Everything else is a literal comparison (case-sensitive, like git).
fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    glob_rec(&p, 0, &n, 0)
}

fn glob_rec(p: &[char], pi: usize, n: &[char], ni: usize) -> bool {
    let mut pi = pi;
    let mut ni = ni;
    loop {
        if pi == p.len() {
            return ni == n.len();
        }
        match p[pi] {
            '*' => {
                // Collapse consecutive stars.
                let mut next = pi + 1;
                while next < p.len() && p[next] == '*' {
                    next += 1;
                }
                for skip in ni..=n.len() {
                    if glob_rec(p, next, n, skip) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if ni == n.len() {
                    return false;
                }
                pi += 1;
                ni += 1;
            }
            ch => {
                if ni == n.len() || n[ni] != ch {
                    return false;
                }
                pi += 1;
                ni += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(text: &str) -> IgnoreRules {
        let mut r = IgnoreRules::default();
        r.add_lines(text);
        r
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let r = rules("# comment\n\n*.log\n");
        assert!(r.is_ignored("debug.log", false));
        assert!(!r.is_ignored("src/main.rs", false));
    }

    #[test]
    fn simple_wildcard_matches_any_depth() {
        let r = rules("*.pyc\n");
        assert!(r.is_ignored("src/a.pyc", false));
        assert!(r.is_ignored("a.pyc", false));
        assert!(!r.is_ignored("src/a.py", false));
    }

    #[test]
    fn directory_pattern_ignores_contents_but_not_same_named_file() {
        let r = rules("build/\n");
        assert!(r.is_ignored("build", true));
        assert!(r.is_ignored("build/out.o", false));
        assert!(!r.is_ignored("src/build", false));
    }

    #[test]
    fn root_anchored_pattern_only_matches_at_root() {
        let r = rules("/dist\n");
        assert!(r.is_ignored("dist", true));
        assert!(r.is_ignored("dist/x.js", false));
        assert!(!r.is_ignored("src/dist", true));
    }

    #[test]
    fn double_star_matches_nested() {
        let r = rules("**/test-output/**\n");
        assert!(r.is_ignored("a/b/test-output/c.txt", false));
        assert!(r.is_ignored("test-output/x", false));
        assert!(!r.is_ignored("src/main.rs", false));

        let r2 = rules("src/**/*.tmp\n");
        assert!(r2.is_ignored("src/a/b/c.tmp", false));
        assert!(r2.is_ignored("src/x.tmp", false));
        assert!(!r2.is_ignored("docs/x.tmp", false));
    }

    #[test]
    fn negation_reincludes() {
        let r = rules("*.log\n!important.log\n");
        assert!(r.is_ignored("debug.log", false));
        assert!(!r.is_ignored("logs/important.log", false));
    }

    #[test]
    fn question_mark_wildcard() {
        let r = rules("file?.txt\n");
        assert!(r.is_ignored("file1.txt", false));
        assert!(!r.is_ignored("file10.txt", false));
    }

    #[test]
    fn question_mark_wildcard_extra() {
        let r = rules("data-?.json\n");
        assert!(r.is_ignored("data-7.json", false));
        assert!(!r.is_ignored("data-77.json", false));
    }

    #[test]
    fn claudeignore_pattern_supported() {
        let r = rules("secrets/\n*.env\n");
        assert!(r.is_ignored("secrets/keys.txt", false));
        assert!(r.is_ignored(".env", false));
        assert!(r.is_ignored("config/.env", false));
    }
}
