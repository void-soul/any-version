use super::*;

// ─── 版本比较（轻量，满足 >=20 之类约束）───

pub(super) fn parse_version_parts(v: &str) -> Vec<u64> {
    let mut parts = Vec::new();
    let mut current = 0u64;
    let mut has_digit = false;
    for c in v.chars() {
        if c.is_ascii_digit() {
            current = current * 10 + (c as u64 - '0' as u64);
            has_digit = true;
        } else if has_digit {
            parts.push(current);
            current = 0;
            has_digit = false;
        }
    }
    if has_digit {
        parts.push(current);
    }
    parts
}

pub(super) fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let pa = parse_version_parts(a);
    let pb = parse_version_parts(b);
    let len = pa.len().max(pb.len());
    for i in 0..len {
        let va = pa.get(i).copied().unwrap_or(0);
        let vb = pb.get(i).copied().unwrap_or(0);
        match va.cmp(&vb) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// 判断 version 是否满足形如 `>=20` / `>20` / `<=18` / `<20` / `20` 的约束。
/// 空约束返回 true。
pub(super) fn version_satisfies(version: &str, requirement: &str) -> bool {
    let req = requirement.trim();
    if req.is_empty() {
        return true;
    }
    let version = version.trim();
    let (op, target) = if let Some(rest) = req.strip_prefix(">=") {
        (">=", rest.trim())
    } else if let Some(rest) = req.strip_prefix("<=") {
        ("<=", rest.trim())
    } else if let Some(rest) = req.strip_prefix('>') {
        (">", rest.trim())
    } else if let Some(rest) = req.strip_prefix('<') {
        ("<", rest.trim())
    } else if let Some(rest) = req.strip_prefix('=') {
        ("=", rest.trim())
    } else {
        // 裸版本号（如 "20"）视为「最低版本」语义，即 >=20
        (">=", req)
    };
    let cmp = compare_versions(version, target);
    match op {
        ">=" => cmp != std::cmp::Ordering::Less,
        "<=" => cmp != std::cmp::Ordering::Greater,
        ">" => cmp == std::cmp::Ordering::Greater,
        "<" => cmp == std::cmp::Ordering::Less,
        _ => cmp == std::cmp::Ordering::Equal,
    }
}
