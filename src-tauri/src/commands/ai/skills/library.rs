//! library responsibility.
use super::*;

// ─── 技能元数据（分类/标签）读写 ───
// 与 skills.sh 规范解耦：不污染 SKILL.md，单独存于点文件 .meta.json。

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub(super) struct SkillMetaFile {
    #[serde(default)]
    pub(super) skills: HashMap<String, SkillMetaItem>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(super) struct SkillMetaItem {
    #[serde(default)]
    pub(super) category: String,
    #[serde(default)]
    pub(super) tags: Vec<String>,
}

/// 元数据文件路径：`<技能库>/.meta.json`（点文件，扫描时跳过）
pub(super) fn skill_meta_path() -> PathBuf {
    skills_dir().join(".meta.json")
}

pub(super) fn load_skill_meta() -> SkillMetaFile {
    let path = skill_meta_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(meta) = serde_json::from_str::<SkillMetaFile>(&data) {
                return meta;
            }
        }
    }
    SkillMetaFile::default()
}

pub(super) fn save_skill_meta(meta: &SkillMetaFile) -> Result<(), String> {
    let data = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(&skill_meta_path(), data.as_bytes())
}

// ─── SKILL.md 解析 ───

pub(super) fn strip_quotes(s: &str) -> String {
    let t = s.trim();
    if t.len() < 2 {
        return t.to_string();
    }
    match (t.chars().next(), t.chars().last()) {
        (Some('"'), Some('"')) | (Some('\''), Some('\'')) => t[1..t.len() - 1].to_string(),
        _ => t.to_string(),
    }
}

pub(super) fn parse_skill_md(content: &str, folder_name: &str) -> (String, String) {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map_or(false, |l| l.trim() == "---") {
        if let Some(end) = lines.iter().skip(1).position(|l| l.trim() == "---") {
            let mut name: Option<String> = None;
            let mut desc: Option<String> = None;
            for line in &lines[1..=end] {
                let line = line.trim();
                if name.is_none() {
                    if let Some(v) = line.strip_prefix("name:") {
                        name = Some(strip_quotes(v));
                        if desc.is_some() {
                            break;
                        }
                        continue;
                    }
                }
                if desc.is_none() {
                    if let Some(v) = line.strip_prefix("description:") {
                        desc = Some(strip_quotes(v));
                        if name.is_some() {
                            break;
                        }
                    }
                }
            }
            let name = name.unwrap_or_else(|| folder_name.to_string());
            let desc = desc.unwrap_or_default();
            return (name, desc);
        }
    }
    let desc = lines
        .first()
        .unwrap_or(&"")
        .trim_start_matches('#')
        .trim()
        .to_string();
    (folder_name.to_string(), desc)
}

// ─── 数据结构 ───

/// 技能条目（简化版总览）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub installed_at: String,
    pub install_method: String,
    /// 用户自定义分类（来自 .meta.json，缺省空字符串）
    #[serde(default)]
    pub category: String,
    /// 用户自定义标签（来自 .meta.json，缺省空数组）
    #[serde(default)]
    pub tags: Vec<String>,
    /// SKILL.md 的字符数。UI 拿它一眼看出「这个技能有多重」——
    /// 描述在每次会话都常驻，而正文只在用到时读，字符数是这个技能的成本指标。
    pub skill_md_chars: usize,
}

/// 安装进度事件
#[derive(Serialize, Clone, Debug)]
pub struct SkillInstallProgress {
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub skill_name: String,
    pub message: String,
}

pub(super) fn emit_install_progress(
    app: &tauri::AppHandle,
    stage: &str,
    current: usize,
    total: usize,
    skill_name: &str,
    message: &str,
) {
    let _ = app.emit(
        "skill-install-progress",
        SkillInstallProgress {
            stage: stage.to_string(),
            current,
            total,
            skill_name: skill_name.to_string(),
            message: message.to_string(),
        },
    );
}

/// 判断路径是否为 junction/symlink
pub(super) fn is_junction(path: &PathBuf) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

// ─── 技能列表（仓库扫描） ───

/// 技能总览：扫描技能库目录（默认「数据目录下的 skills」）。
/// 手工拷进去的技能同样会被列出来——它和安装进来的技能没有区别对待。
#[tauri::command]
pub fn get_skill_overview() -> Result<Vec<SkillEntry>, String> {
    let store = skills_dir();
    // 全局 manifest / meta 按 id 关联（与具体仓库目录无关）
    let manifest = load_skills();
    let meta = load_skill_meta();
    let mut entries: Vec<SkillEntry> = Vec::new();

    if !store.exists() {
        return Ok(entries);
    }

    let dir_entries = fs::read_dir(&store).map_err(|e| e.to_string())?;
    for entry in dir_entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if id.starts_with('.') {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        let (name, description, skill_md_chars) = if skill_md.is_file() {
            let c = fs::read_to_string(&skill_md).unwrap_or_default();
            let (n, d) = parse_skill_md(&c, &id);
            (n, d, c.chars().count())
        } else {
            (id.clone(), String::new(), 0)
        };

        // 从 manifest 补充元数据
        let (installed_at, install_method) = manifest
            .skills
            .iter()
            .find(|s| s.id == id)
            .map(|s| (s.installed_at.clone(), s.install_method.clone()))
            .unwrap_or_default();

        // 从 .meta.json 补充用户分类/标签
        let m = meta.skills.get(&id);
        let category = m.map(|x| x.category.clone()).unwrap_or_default();
        let tags = m.map(|x| x.tags.clone()).unwrap_or_default();

        entries.push(SkillEntry {
            id,
            name,
            description,
            installed_at,
            install_method,
            category,
            tags,
            skill_md_chars,
        });
    }

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(entries)
}

/// 返回全部技能元数据（分类/标签），用于前端构建筛选选项
#[tauri::command]
pub fn get_skill_meta() -> Result<HashMap<String, SkillMetaItemDto>, String> {
    let meta = load_skill_meta();
    let mut out: HashMap<String, SkillMetaItemDto> = HashMap::new();
    for (k, v) in meta.skills {
        out.insert(
            k,
            SkillMetaItemDto {
                category: v.category,
                tags: v.tags,
            },
        );
    }
    Ok(out)
}

/// 更新技能的分类与标签（写入 .meta.json）
#[tauri::command]
pub fn update_skill_meta(
    skill_id: String,
    category: String,
    tags: Vec<String>,
) -> Result<(), String> {
    if skill_id.is_empty() {
        return Err("技能 id 不能为空".to_string());
    }
    let mut meta = load_skill_meta();
    let tags: Vec<String> = tags
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    meta.skills.insert(
        skill_id.clone(),
        SkillMetaItem {
            category: category.trim().to_string(),
            tags,
        },
    );
    save_skill_meta(&meta)
}

/// 元数据项 DTO（前端筛选用）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillMetaItemDto {
    pub category: String,
    pub tags: Vec<String>,
}

// ─── 克隆来源解析（子目录标记 / 技能根定位 / 临时目录） ───

/// 拆分来源里的子目录标记：`owner/repo#skills/foo` → (`owner/repo`, Some("skills/foo"))。
///
/// 一个仓库里放多个技能时需要指定装哪个；也是「技能 id 与目录名不一致」时的补救入口
/// （参考 cc-switch「support mismatched skillId directories / preserve source path during updates」）。
pub(crate) fn split_source_subdir(source: &str) -> (String, Option<String>) {
    match source.split_once('#') {
        Some((base, sub)) => {
            let base = base.trim();
            let sub = sub.trim().trim_matches('/');
            if base.is_empty() || sub.is_empty() {
                (source.to_string(), None)
            } else {
                (base.to_string(), Some(sub.to_string()))
            }
        }
        None => (source.to_string(), None),
    }
}

/// SKILL.md 的最深搜索层级：仓库根 → `*/` → `*/*`。
pub(super) const SKILL_ROOT_MAX_DEPTH: usize = 2;
