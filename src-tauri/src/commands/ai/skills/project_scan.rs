//! project scan responsibility.
use super::*;

/// 项目里已安装的技能（扫描推断，不持久化）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillEntry {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    /// `linked` 指向技能库的链接 · `local` 项目自带的真实目录 · `broken` 悬空链接（技能库里的技能已被删除）
    pub state: String,
    /// SKILL.md 字符数（与技能库列表同一指标，便于两边对照）
    pub skill_md_chars: usize,
    /// 技能库 `.meta.json` 里设的标签 —— 项目列表要跟着显示，
    /// 否则用户在技能库整理的分类/标签到了项目里就全丢了。
    pub tags: Vec<String>,
    pub category: String,
    /// 装在**哪些扫描目录**（`.agents` / `.codebuddy` / …）。
    ///
    /// 一个技能装到 N 个目录就有 N 个 junction，但**只列一行** —— 否则用户会看到
    /// 同一个技能重复 5 次，还以为装坏了。
    pub dirs: Vec<String>,
    /// 这些链接里有多少已经失效（目标技能被从库里删了）。
    ///
    /// 只要还有一个链接有效就算「已安装」：5 个里坏 1 个不该让用户以为出了大事，
    /// 但也不能完全吞掉——否则用户永远不知道自己该去清理哪几个。
    pub stale_links: usize,
}

/// 项目里所有扫描目录下的 skills 路径（供「装在哪」这类问题用）
pub(super) fn project_skill_dirs_of(project_id: &str) -> Result<Vec<PathBuf>, String> {
    project_skill_dirs_in(&projects_path(), project_id)
}

/// 取项目里某个技能的 SKILL.md 全文，供 UI 渲染预览。
///
/// **按需取**而不是随列表一起返回：一份 SKILL.md 动辄几千字符，几十个技能
/// 一起塞进列表会把 IPC 撑大，而用户通常一次只看一个。
#[tauri::command]
pub fn get_project_skill_md(project_id: String, skill_id: String) -> Result<String, String> {
    // 技能可能装在任意一个扫描目录下（甚至几个），按注册顺序找第一个存在的
    for dir in project_skill_dirs_of(&project_id)? {
        let path = dir.join(&skill_id).join("SKILL.md");
        if path.is_file() {
            // junction 可穿透：读到的是技能库里的实体文件
            return fs::read_to_string(&path)
                .map_err(|e| format!("{} 读取失败: {}", path.display(), e));
        }
    }
    Err(format!("项目里找不到技能「{}」的 SKILL.md", skill_id))
}

/// 取技能库里某个技能的 SKILL.md 全文，供 UI 渲染预览。
///
/// 与 `get_project_skill_md` 分开而不是合并成一个带可选参数的命令：两个入口的
/// 失败语义不同（项目里可能是悬空链接，技能库里是真的没这个技能），分开报错更准。
#[tauri::command]
pub fn get_library_skill_md(skill_id: String) -> Result<String, String> {
    let path = skills_dir().join(&skill_id).join("SKILL.md");
    fs::read_to_string(&path).map_err(|e| format!("{} 读取失败: {}", path.display(), e))
}

/// 列出某个项目装了什么：扫**所有扫描目录**下的 skills，看哪些是指向技能库的 junction。
///
/// 状态不落盘 —— 清单与实况必然会漂移（用户手动删链接、技能库删技能、移动项目），
/// 扫描推断则永远等于磁盘真实状态。
///
/// 关键：**按 skill_id 去重**。一个技能装到 N 个目录就有 N 个 junction，
/// 但它是**一个**技能 —— 不去重的话用户会看到同一行重复 5 次，还以为装坏了。
#[tauri::command]
pub fn list_project_skills(project_id: String) -> Result<Vec<ProjectSkillEntry>, String> {
    list_installed(&skills_dir(), &projects_path(), &project_id)
}

pub(super) fn list_installed(
    store: &Path,
    projects_file: &Path,
    project_id: &str,
) -> Result<Vec<ProjectSkillEntry>, String> {
    let mut out: Vec<ProjectSkillEntry> = Vec::new();
    // .meta.json 读一次就够，别在循环里反复读盘
    let meta = load_skill_meta();
    // 每个扫描目录扫一遍，逐条并入 out（按 skill_id 合并）
    for (dir_name, dir) in SKILL_SCAN_DIRS
        .iter()
        .zip(project_skill_dirs_in(projects_file, project_id)?)
    {
        // 项目还没装过任何技能（目录不存在）：跳过而不是报错
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            scan_one_into(store, &dir, dir_name, entry.path(), &meta, &mut out)?;
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

/// 把一个目录条目并入结果（已存在同 id 则合并 `dirs` / `stale_links`）
pub(super) fn scan_one_into(
    store: &Path,
    _dir: &Path,
    dir_name: &str,
    path: PathBuf,
    meta: &SkillMetaFile,
    out: &mut Vec<ProjectSkillEntry>,
) -> Result<(), String> {
    let fallback_id = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if fallback_id.starts_with('.') {
        return Ok(());
    }
    let is_link = fs::symlink_metadata(&path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);

    if !is_link {
        // 真实目录 = 项目**自带**的技能（用户手工维护、没进技能库）。
        // 同样要认出来：不然它在 UI 上完全隐形，用户既看不到也清不掉。
        // 名字/描述就地读它自己的 SKILL.md，不去技能库找。
        let md = path.join("SKILL.md");
        let (name, description, chars) = if md.is_file() {
            let c = fs::read_to_string(&md).unwrap_or_default();
            let (n, d) = parse_skill_md(&c, &fallback_id);
            (n, d, c.chars().count())
        } else {
            (fallback_id.clone(), String::new(), 0)
        };
        push_or_merge(
            out,
            ProjectSkillEntry {
                skill_id: fallback_id.clone(),
                name,
                description,
                state: "local".to_string(),
                skill_md_chars: chars,
                // 项目自带技能不在技能库里，没有 .meta.json 可查
                tags: Vec::new(),
                category: String::new(),
                dirs: vec![dir_name.to_string()],
                stale_links: 0,
            },
        );
        return Ok(());
    }

    let target_id = junction_target_skill(&path, store);
    let (skill_id, alive) = match target_id {
        Some(id) => {
            // 先算存活再交出所有权：tuple 里直接 move 会让后面的借用报错
            let alive = store.join(&id).is_dir();
            (id, alive)
        }
        // 指错了地方（不是指向技能库）：按悬空处理，UI 会标出来并给清理入口
        None => (fallback_id.clone(), false),
    };
    let (name, description, chars) = if alive {
        let md = store.join(&skill_id).join("SKILL.md");
        if md.is_file() {
            let c = fs::read_to_string(&md).unwrap_or_default();
            let (n, d) = parse_skill_md(&c, &skill_id);
            (n, d, c.chars().count())
        } else {
            (skill_id.clone(), String::new(), 0)
        }
    } else {
        (fallback_id.clone(), String::new(), 0)
    };
    // 标签/分类从技能库的 .meta.json 取：用户在技能库整理的元数据
    // 在项目列表里也要看得见，否则两边的信息会各说各话。
    let m = meta.skills.get(&skill_id);
    let tags = m.map(|x| x.tags.clone()).unwrap_or_default();
    let category = m.map(|x| x.category.clone()).unwrap_or_default();
    push_or_merge(
        out,
        ProjectSkillEntry {
            skill_id,
            name,
            description,
            state: if alive { "linked" } else { "broken" }.to_string(),
            skill_md_chars: chars,
            tags,
            category,
            dirs: vec![dir_name.to_string()],
            // 这个链接本身失效时才算数；同一技能在别的目录还有效 → 合并后仍是「已安装」
            stale_links: usize::from(!alive),
        },
    );
    Ok(())
}

/// 并入结果：同 id 已存在时合并目录与失效计数，**且任一链接有效即算已安装**。
pub(super) fn push_or_merge(out: &mut Vec<ProjectSkillEntry>, item: ProjectSkillEntry) {
    if let Some(existing) = out.iter_mut().find(|e| e.skill_id == item.skill_id) {
        // `dirs` 只记**有效**安装位置；失效链接只进stale_links，
        // 否则「装在哪」里会混着已经坏掉的路径，用户照着去找却找不到
        if item.stale_links == 0 {
            for d in item.dirs {
                if !existing.dirs.contains(&d) {
                    existing.dirs.push(d);
                }
            }
        }
        existing.stale_links += item.stale_links;
        // broken 只在「所有链接都失效」时才成立
        if existing.state == "broken" && item.state == "linked" {
            existing.state = "linked".to_string();
            existing.name = item.name;
            existing.description = item.description;
            existing.skill_md_chars = item.skill_md_chars;
            existing.tags = item.tags;
            existing.category = item.category;
        }
        return;
    }
    out.push(item);
}

// ─── 项目技能计数（一次扫完所有项目） ───

/// 单个项目的技能计数
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillCount {
    pub project_id: String,
    /// 有效技能总数（= linked + local，**不含** broken）
    pub total: usize,
    /// 来自技能库（junction 指到某个库里真实存在的技能）
    pub linked: usize,
    /// 项目自带的真实目录（用户手工维护、没进技能库）
    pub local: usize,
    /// 悬空链接：是指向技能库的链接，但目标已不在任何库里（最容易被漏掉的一类）
    pub broken: usize,
}

/// 一次扫完所有项目的技能计数。
///
/// 与 `list_project_skills` 的区别是**只数、不读**：不碰 `SKILL.md`，不解析名称/描述/字符数。
/// 项目列表里每行都要显示技能数，如果用完整清单就得为每个项目的每个技能读一次盘——
/// 几十个项目就是上百次读盘，只为了拿一个数字。
///
/// 判定口径与 `list_installed` 严格一致（含「非点目录一律当项目自带」这条），
/// 否则列表上显示的数字会和点进去看到的条目数对不上。
#[tauri::command]
pub fn list_project_skill_counts() -> Vec<ProjectSkillCount> {
    count_project_skills(&projects_path(), &skills_dir())
}

/// 可注入的版本（单测必须能指向临时 projects 文件，否则会读用户真实清单）
pub(super) fn count_project_skills(projects_file: &Path, store: &Path) -> Vec<ProjectSkillCount> {
    let mut out: Vec<ProjectSkillCount> = Vec::new();
    for p in load_projects_from(projects_file).projects {
        // 跨扫描目录按 id 归并：技能装到 N 个目录就有 N 个 junction，
        // 不去重的话「项目技能数」会直接翻 N 倍（这正是引入多目录后最容易漏的一处）
        let mut linked_ids: HashSet<String> = HashSet::new();
        let mut broken_ids: HashSet<String> = HashSet::new();
        let mut local_ids: HashSet<String> = HashSet::new();
        for dir_name in SKILL_SCAN_DIRS {
            let dir = Path::new(&p.path).join(dir_name).join("skills");
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let path = entry.path();
                let is_link = fs::symlink_metadata(&path)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if !is_link {
                    // 真实目录 = 项目自带（与 list_installed 同口径：不额外要求 is_dir，
                    // 否则这里数出来的会比点进去看到的条目数少）
                    local_ids.insert(name);
                    continue;
                }
                // 是链接：真实目标还落在库里才算 linked，否则算悬空
                match junction_target_skill(&path, store) {
                    Some(id) if store.join(&id).is_dir() => {
                        linked_ids.insert(id);
                    }
                    _ => {
                        broken_ids.insert(name);
                    }
                }
            }
        }
        // 同一技能既有有效链接又有失效链接时按「已安装」算（与列表口径一致）
        broken_ids.retain(|id| !linked_ids.contains(id));
        out.push(ProjectSkillCount {
            project_id: p.id,
            total: linked_ids.len() + local_ids.len(),
            linked: linked_ids.len(),
            local: local_ids.len(),
            broken: broken_ids.len(),
        });
    }
    out
}
