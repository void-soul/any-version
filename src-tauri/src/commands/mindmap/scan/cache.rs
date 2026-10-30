use super::*;

pub(super) fn scan_cache_dir() -> std::path::PathBuf {
    crate::commands::config::get_data_dir().join("scan-cache")
}

/// 计算单个条目的指纹贡献：相对路径 + mtime + 大小。
/// 文件/目录都参与——目录 mtime 变化也能捕捉到无文件增删的改名场景。
pub(super) fn fingerprint_entry(out: &mut String, root: &Path, rel: &str) {
    let meta = std::fs::metadata(root.join(rel));
    match meta {
        Ok(m) => {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            out.push_str(&format!("{}|{}|{}\n", rel, mtime, m.len()));
        }
        Err(_) => out.push_str(&format!("{}|gone\n", rel)),
    }
}

/// 扫描输入指纹：覆盖「会影响扫描产出」的全部输入——
/// 扫描器版本、规范化路径、条目集（含每个条目的 mtime/大小）、ignore 规则文件内容。
/// 注意不含 user_hint：提示语是拼在扫描文本之后的，缓存的是纯扫描产出。
pub(super) fn scan_fingerprint(root: &Path, entries: &[Entry]) -> String {
    let mut body = format!("v{}\n", SCAN_CACHE_VERSION);
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    body.push_str(&canon.to_string_lossy());
    body.push('\n');
    for e in entries {
        fingerprint_entry(&mut body, root, &e.rel);
    }
    // ignore 规则内容参与指纹：规则变更（哪怕条目集恰好不变）也作废缓存
    for name in [".gitignore", ".claudeignore", ".git/info/exclude"] {
        if let Ok(data) = std::fs::read_to_string(root.join(name)) {
            body.push_str(&format!("--{name}\n{data}"));
        }
    }
    format!("{:x}", md5::compute(body.as_bytes()))
}

pub(super) fn read_scan_cache(fp: &str) -> Option<String> {
    let path = scan_cache_dir().join(format!("{}.json", fp));
    let data = std::fs::read_to_string(&path).ok()?;
    let entry: ScanCacheEntry = serde_json::from_str(&data).ok()?;
    if entry.fingerprint != fp {
        return None;
    }
    Some(entry.context)
}

pub(super) fn write_scan_cache(fp: &str, context: &str) {
    let dir = scan_cache_dir();
    if crate::commands::config::atomic_write_file(
        &dir.join(format!("{}.json", fp)),
        serde_json::to_vec(&ScanCacheEntry {
            fingerprint: fp.to_string(),
            saved_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            context: context.to_string(),
        })
        .unwrap_or_default()
        .as_slice(),
    )
    .is_err()
    {
        return; // 缓存写失败不影响主流程
    }
    prune_scan_cache();
}

/// 上限淘汰：缓存份数超过 SCAN_CACHE_MAX_ENTRIES 时按 saved_at 删最旧的。
pub(super) fn prune_scan_cache() {
    let dir = scan_cache_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut items: Vec<(std::path::PathBuf, u128)> = rd
        .filter_map(|e| {
            let p = e.ok()?.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                return None;
            }
            let data = std::fs::read_to_string(&p).ok()?;
            let v: serde_json::Value = serde_json::from_str(&data).ok()?;
            let ts = v.get("savedAtMs").and_then(|x| x.as_u64()).unwrap_or(0) as u128;
            Some((p, ts))
        })
        .collect();
    if items.len() <= SCAN_CACHE_MAX_ENTRIES {
        return;
    }
    items.sort_by_key(|(_, ts)| *ts);
    let extra = items.len() - SCAN_CACHE_MAX_ENTRIES;
    for (p, _) in items.into_iter().take(extra) {
        let _ = std::fs::remove_file(p);
    }
}

/// 带缓存的项目扫描：指纹命中且缓存可读时直接回放上次扫描文本（微秒级），
/// 否则全盘扫描后写缓存。指纹覆盖 scanner 版本/路径/条目集 mtime+size/ignore 规则，
/// 任何会影响产出的变更都会自动失效。user_hint 不参与指纹（拼接在缓存文本之后）。
pub fn scan_project_with_hint(path: &str, user_hint: Option<&str>) -> Result<String, String> {
    let root = Path::new(path);
    if !root.is_dir() {
        return Err(format!("项目目录不存在: {}", path));
    }
    // 指纹计算需要条目集；为避免「先收集一次算指纹、未命中再收集一次」的双重遍历，
    // 直接复用同一份条目集：collect_entries 是扫描的第一步，代价可接受。
    let entries = collect_entries(root);
    let fp = scan_fingerprint(root, &entries);
    if let Some(cached) = read_scan_cache(&fp) {
        return Ok(append_hint(cached, user_hint));
    }
    let context = scan_project_from_entries(path, root, &entries)?;
    write_scan_cache(&fp, &context);
    Ok(append_hint(context, user_hint))
}

pub(super) fn append_hint(context: String, user_hint: Option<&str>) -> String {
    let mut ctx = context;
    if let Some(hint) = user_hint {
        let hint = hint.trim();
        if !hint.is_empty() {
            ctx.push_str(&format!("\n\n## 用户附加说明（必须遵守）\n{}", hint));
        }
    }
    ctx
}
