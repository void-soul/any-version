use super::{
    // count_project_skills / junction_target_skill 来自「项目技能计数」那个提交：
    // 收回单库后它们仍在，只是签名回到单 store
    add_project_at,
    count_project_skills,
    default_skills_dir,
    find_skill_root,
    install_into,
    junction_target_skill,
    list_installed,
    load_projects_from,
    remove_project_at,
    source_to_url,
    split_source_subdir,
    uninstall_from,
    PRESET_SKILL_SOURCES,
    SKILL_SCAN_DIRS,
};

/// 取项目根目录（安装后要断言 junction 落在哪）
fn project_root(file: &Path, pid: &str) -> PathBuf {
    PathBuf::from(
        load_projects_from(file)
            .projects
            .into_iter()
            .find(|p| p.id == pid)
            .expect("项目应在清单里")
            .path,
    )
}
use std::path::{Path, PathBuf};

/// `owner/repo` 要补成完整 URL 才能喂给 `git ls-remote`；已是 URL 的不能被二次拼接。
#[test]
fn source_url_normalizes_repo_shorthand() {
    assert_eq!(
        source_to_url("openai/skills"),
        "https://github.com/openai/skills"
    );
    assert_eq!(
        source_to_url("https://github.com/anthropics/skills"),
        "https://github.com/anthropics/skills"
    );
}

/// 预置源里不能出现重复仓库（前端是按钮列表，重复会出两个一样的按钮）。
#[test]
fn preset_sources_are_unique_and_well_formed() {
    let mut repos: Vec<&str> = PRESET_SKILL_SOURCES.iter().map(|(_, r, _)| *r).collect();
    let total = repos.len();
    repos.sort_unstable();
    repos.dedup();
    assert_eq!(repos.len(), total, "预置源有重复仓库");
    for (label, repo, hint) in PRESET_SKILL_SOURCES {
        assert!(
            !label.is_empty() && !hint.is_empty(),
            "{repo} 缺少标签或说明"
        );
        assert_eq!(repo.split('/').count(), 2, "{repo} 应是 owner/repo 形态");
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("anyver-skills-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_skill(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: demo\ndescription: d\n---\n",
    )
    .unwrap();
}

#[test]
fn split_source_subdir_parses_optional_subdir() {
    assert_eq!(
        split_source_subdir("owner/repo"),
        ("owner/repo".to_string(), None)
    );
    assert_eq!(
        split_source_subdir("owner/repo#skills/foo"),
        ("owner/repo".to_string(), Some("skills/foo".to_string()))
    );
    // 只有一侧有内容时按普通来源处理，避免把来源拆坏
    assert_eq!(split_source_subdir("#x"), ("#x".to_string(), None));
    assert_eq!(
        split_source_subdir("owner/repo#"),
        ("owner/repo#".to_string(), None)
    );
}

/// 仓库根就是技能：照旧直接用根目录。
#[test]
fn skill_root_prefers_repository_root() {
    let dir = temp_dir("root");
    write_skill(&dir);
    assert_eq!(find_skill_root(&dir, None).unwrap(), dir);
}

/// 仓库根没有 SKILL.md 时要下钻找到真正的技能目录（此前会退化成临时目录名）。
#[test]
fn skill_root_finds_single_nested_skill() {
    let dir = temp_dir("nested");
    write_skill(&dir.join("skills").join("ppt-master"));
    let root = find_skill_root(&dir, None).unwrap();
    assert!(
        root.ends_with("skills/ppt-master"),
        "实际: {}",
        root.display()
    );
    // 技能目录名才是技能 id 的来源
    assert_eq!(root.file_name().unwrap().to_string_lossy(), "ppt-master");
}

#[test]
fn skill_root_accepts_explicit_subdir_and_rejects_missing_one() {
    let dir = temp_dir("explicit");
    write_skill(&dir.join("skills").join("alpha"));
    let root = find_skill_root(&dir, Some("skills/alpha")).unwrap();
    assert!(root.ends_with("skills/alpha"));
    assert!(find_skill_root(&dir, Some("skills/missing")).is_err());
}

/// 多技能仓库必须报错并让用户用 `#子目录` 指定，而不是随便装一个。
#[test]
fn skill_root_reports_ambiguous_repositories() {
    let dir = temp_dir("ambiguous");
    write_skill(&dir.join("skills").join("alpha"));
    write_skill(&dir.join("skills").join("beta"));
    let err = find_skill_root(&dir, None).unwrap_err();
    assert!(
        err.contains("alpha") && err.contains("beta"),
        "错误信息应列出候选: {}",
        err
    );
    assert!(find_skill_root(&dir, Some("skills/beta")).is_ok());
}

#[test]
fn skill_root_rejects_non_skill_repositories() {
    let dir = temp_dir("plain");
    std::fs::write(dir.join("README.md"), "no skill").unwrap();
    assert!(find_skill_root(&dir, None).is_err());
}

// ─── 项目（安装作用域） ───

/// 设计不变量：默认技能库**不能**是 `~/.agents/skills`。
///
/// 那是 claude-code / codex-cli / opencode / cursor / cline / firebender 的原生扫描目录，
/// 放进去的技能会对**所有**项目全局生效，每次会话都注入全部 name + description。
/// 这条测试是为了防止有人日后"顺手改回默认值"。
#[test]
fn default_skills_dir_avoids_native_agent_scan_dir() {
    let d = default_skills_dir().to_string_lossy().replace('\\', "/");
    assert!(
        !d.ends_with("/.agents/skills"),
        "默认技能库不能落在原生扫描目录：{}",
        d
    );
    assert!(
        d.ends_with("/skills"),
        "默认技能库应是数据目录下的 skills：{}",
        d
    );
}

#[test]
fn add_project_rejects_empty_and_non_directory() {
    let base = temp_dir("add-invalid");
    let file = base.join("projects.json");
    assert!(add_project_at(&file, "   ").is_err(), "空路径应被拒");
    let not_dir = base.join("a.txt");
    std::fs::write(&not_dir, "x").unwrap();
    assert!(
        add_project_at(&file, &not_dir.to_string_lossy()).is_err(),
        "文件不是目录，应被拒"
    );
    assert!(!file.exists(), "被拒时不应留下 projects.json");
}

#[test]
fn add_project_persists_and_dedupes() {
    let base = temp_dir("add-ok");
    let file = base.join("projects.json");
    let proj = base.join("my-app");
    std::fs::create_dir_all(&proj).unwrap();

    let p = add_project_at(&file, &proj.to_string_lossy()).expect("应添加成功");
    assert_eq!(p.name, "my-app");
    assert!(!p.id.is_empty(), "项目必须有 id");

    // 重新从文件读回：验证落盘 + 反序列化
    let loaded = load_projects_from(&file);
    assert_eq!(loaded.projects.len(), 1);
    assert_eq!(loaded.projects[0].id, p.id);

    // 同一目录再添一次必须被拒
    assert!(add_project_at(&file, &proj.to_string_lossy()).is_err());
    // 多一个尾分隔符仍是同一路径，也应被拒
    let with_sep = format!("{}{}", proj.to_string_lossy(), std::path::MAIN_SEPARATOR);
    assert!(add_project_at(&file, &with_sep).is_err());
}

#[test]
fn remove_project_only_touches_manifest() {
    let base = temp_dir("rm-project");
    let file = base.join("projects.json");
    let proj = base.join("app");
    std::fs::create_dir_all(&proj).unwrap();
    let p = add_project_at(&file, &proj.to_string_lossy()).unwrap();

    assert!(
        remove_project_at(&file, "no-such-id").is_err(),
        "删不存在的 id 应报错"
    );
    remove_project_at(&file, &p.id).unwrap();
    assert!(load_projects_from(&file).projects.is_empty());
    assert!(proj.is_dir(), "移除项目条目不应删除项目目录");
}

// ─── 安装到项目 ───

/// 建一套隔离环境：技能库 store + 项目 proj + 项目清单，返回 (base, store, 清单, 项目id)。
fn setup_project_env(name: &str) -> (PathBuf, PathBuf, PathBuf, String) {
    let base = temp_dir(name);
    let store = base.join("store");
    std::fs::create_dir_all(&store).unwrap();
    let proj = base.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let file = base.join("projects.json");
    let p = add_project_at(&file, &proj.to_string_lossy()).unwrap();
    (base, store, file, p.id)
}

fn make_skill(store: &Path, id: &str) {
    let d = store.join(id);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("SKILL.md"),
        format!("---\nname: {}\ndescription: demo {}\n---\n", id, id),
    )
    .unwrap();
}

#[test]
fn install_rejects_unknown_skill_or_project() {
    let (_base, store, file, pid) = setup_project_env("unknown");
    make_skill(&store, "demo");
    assert!(
        install_into(&store, &file, "nope", &pid, None).is_err(),
        "技能库没有该技能"
    );
    assert!(
        install_into(&store, &file, "demo", "no-such-project", None).is_err(),
        "项目不存在应报错"
    );
}

/// 一个技能装到多个扫描目录后：**列表只列一行**、计数也只算 1。
///
/// 这是引入多目录后最容易漏的一处：磁盘上有 N 个 junction，不去重的话
/// 用户会看到同一个技能重复 N 行、项目技能数直接翻 N 倍。
#[cfg(windows)]
#[test]
fn multi_dir_install_is_deduped_in_list_and_count() {
    let (_base, store, file, pid) = setup_project_env("multidir");
    make_skill(&store, "demo");
    let picked = vec![
        ".agents".to_string(),
        ".codebuddy".to_string(),
        ".claude".to_string(),
    ];
    let outcome = install_into(&store, &file, "demo", &pid, Some(&picked)).expect("应安装成功");
    assert_eq!(outcome.linked.len(), 3, "只应装到勾选的 3 个目录");

    // 三个目录里各有一个 junction，指向同一份实体
    let root = project_root(&file, &pid);
    let mut targets = Vec::new();
    for d in &picked {
        let link = root.join(d).join("skills").join("demo");
        assert!(link.is_dir(), "{} 下应有 junction", d);
        targets.push(std::fs::canonicalize(&link).unwrap_or_else(|_| link.clone()));
    }
    assert!(
        targets.windows(2).all(|w| w[0] == w[1]),
        "多个 junction 必须指向同一份实体（否则就变成复制内容了）"
    );

    // 列表：只列一行，且带上装了哪些目录
    let list = list_installed(&store, &file, &pid).unwrap();
    assert_eq!(
        list.len(),
        1,
        "同一技能不该被列成 3 行，实际 {:?}",
        list.iter()
            .map(|e| (&e.skill_id, &e.dirs))
            .collect::<Vec<_>>()
    );
    assert_eq!(list[0].state, "linked");
    assert_eq!(list[0].dirs.len(), 3, "应记录装在 3 个目录");
    assert!(list[0].stale_links == 0);

    // 计数：也是 1
    let c = count_project_skills(&file, &store);
    let c = c.iter().find(|c| c.project_id == pid).unwrap();
    assert_eq!((c.total, c.linked), (1, 1), "计数必须去重");

    // 卸载：所有目录里的链接都清掉
    let removed = uninstall_from(&file, "demo", &pid, None).unwrap();
    assert_eq!(removed.len(), 3);
    for d in &picked {
        assert!(
            !root.join(d).join("skills").join("demo").exists(),
            "{} 下的链接应被清掉",
            d
        );
    }
    assert!(
        store.join("demo").join("SKILL.md").is_file(),
        "绝不能删掉技能库里的实体"
    );
}

/// 逐目录卸载：只去掉指定的那一个，其余目录里的链接**必须留着**。
///
/// 用户的交互是「在技能这一行里点掉 .codebuddy 或 .agents 其中一个」，
/// 所以「只影响指定目录」是这个交互成立的前提 —— 误伤另外两个目录等于
/// 把用户的安装状态改坏了，而它不报错、只是安静地少装了两个。
///
/// 顺带守住「删完最后一个目录，技能就从这个项目里消失」：列表是扫出来的，
/// 目录空了自然不再有这一行。
#[cfg(windows)]
#[test]
fn uninstall_from_one_dir_leaves_the_others_installed() {
    let (_base, store, file, pid) = setup_project_env("onedir");
    make_skill(&store, "demo");
    let picked = vec![".agents".to_string(), ".codebuddy".to_string()];
    install_into(&store, &file, "demo", &pid, Some(&picked)).expect("应安装成功");
    let root = project_root(&file, &pid);

    // 只卸 .codebuddy
    let removed = uninstall_from(&file, "demo", &pid, Some(".codebuddy")).unwrap();
    assert_eq!(
        removed,
        vec![".codebuddy".to_string()],
        "只应报告动过 .codebuddy"
    );
    assert!(
        !root.join(".codebuddy").join("skills").join("demo").exists(),
        ".codebuddy 下的链接应被删掉"
    );
    assert!(
        root.join(".agents").join("skills").join("demo").is_dir(),
        ".agents 下的链接必须原样保留 —— 误伤它等于悄悄改坏用户的安装状态"
    );

    // 列表仍然有这一行（还剩 .agents），且 dirs 只剩一个
    let list = list_installed(&store, &file, &pid).unwrap();
    assert_eq!(list.len(), 1, "还剩一个目录有链接，技能就该仍在列表里");
    assert_eq!(list[0].dirs, vec![".agents".to_string()]);

    // 技能库实体不受影响
    assert!(store.join("demo").join("SKILL.md").is_file());

    // 再卸 .agents：这一次技能就该从项目里彻底消失
    let removed = uninstall_from(&file, "demo", &pid, Some(".agents")).unwrap();
    assert_eq!(removed, vec![".agents".to_string()]);
    assert!(
        list_installed(&store, &file, &pid).unwrap().is_empty(),
        "全部目录都卸完后不该再有这一行"
    );

    // 两个壳目录都空了 -> 逐级删掉
    assert!(!root.join(".agents").exists(), "空的 .agents 应被清掉");
    assert!(
        !root.join(".codebuddy").exists(),
        "空的 .codebuddy 应被清掉"
    );

    let _ = std::fs::remove_dir_all(&_base);
}

/// 目录名必须是扫描目录之一：否则等于让调用方拼任意路径去删东西。
#[cfg(windows)]
#[test]
fn uninstall_rejects_a_dir_that_is_not_a_scan_dir() {
    let (_base, store, file, pid) = setup_project_env("baddir");
    make_skill(&store, "demo");
    let one = vec![".agents".to_string()];
    install_into(&store, &file, "demo", &pid, Some(&one)).unwrap();
    let root = project_root(&file, &pid);
    // 项目里放一个不该被碰的东西
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src").join("main.rs"), "fn main() {}").unwrap();

    let err = uninstall_from(&file, "demo", &pid, Some("src")).unwrap_err();
    assert!(err.contains("不是技能扫描目录"), "got: {}", err);
    assert!(
        root.join("src").join("main.rs").exists(),
        "非扫描目录必须原样不动"
    );
    assert!(root.join(".agents").join("skills").join("demo").is_dir());

    let _ = std::fs::remove_dir_all(&_base);
}

/// 5 个链接里坏 1 个：仍算「已安装」，但要记下失效数。
///
/// 否则用户会以为出了大事（5 个里 4 个 broken），或者完全不知道自己该清理哪个。
#[cfg(windows)]
#[test]
fn stale_link_among_many_still_counts_as_installed() {
    let (_base, store, file, pid) = setup_project_env("stale");
    make_skill(&store, "demo");
    let picked = vec![".agents".to_string(), ".codebuddy".to_string()];
    install_into(&store, &file, "demo", &pid, Some(&picked)).unwrap();

    // 只删掉 .codebuddy 下那个链接，.agents 下的仍然有效
    let link = project_root(&file, &pid)
        .join(".codebuddy")
        .join("skills")
        .join("demo");
    std::fs::remove_dir(&link).unwrap();

    let list = list_installed(&store, &file, &pid).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].state, "linked", "还有一个有效链接就该算已安装");
    assert_eq!(
        list[0].dirs,
        vec![".agents".to_string()],
        "失效的目录不该算在 dirs 里"
    );

    let c = count_project_skills(&file, &store);
    let c = c.iter().find(|c| c.project_id == pid).unwrap();
    assert_eq!((c.total, c.linked, c.broken), (1, 1, 0));
}

/// 安装 → 幂等 → 卸载 → 悬空识别，一条龙。
///
/// 依赖真实 junction（Windows `mklink /J`），故限定 Windows。
#[cfg(windows)]
#[test]
fn project_skill_lifecycle() {
    let (_base, store, file, pid) = setup_project_env("lifecycle");
    make_skill(&store, "demo");

    // 没装过时列表为空，且**不报错**（.agents/skills 还不存在）
    assert!(list_installed(&store, &file, &pid).unwrap().is_empty());

    let outcome = install_into(&store, &file, "demo", &pid, None).expect("应安装成功");
    let link = project_root(&file, &pid)
        .join(".agents")
        .join("skills")
        .join("demo");
    assert_eq!(
        outcome.linked.len(),
        SKILL_SCAN_DIRS.len(),
        "默认应装到**全部**扫描目录，否则用户会发现某个工具不认这个技能"
    );
    assert!(
        Path::new(&link).join("SKILL.md").is_file(),
        "junction 应能穿透读到 SKILL.md"
    );

    let list = list_installed(&store, &file, &pid).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].state, "linked");
    assert_eq!(list[0].name, "demo", "名称应来自 SKILL.md 的 frontmatter");

    // 幂等：重复安装不报错、列表不重复
    install_into(&store, &file, "demo", &pid, None).expect("重复安装应幂等");
    assert_eq!(list_installed(&store, &file, &pid).unwrap().len(), 1);

    // 卸载：链接消失，**技能库内容必须还在**
    uninstall_from(&file, "demo", &pid, None).unwrap();
    assert!(!Path::new(&link).exists(), "链接应被删除");
    assert!(
        store.join("demo").join("SKILL.md").is_file(),
        "卸载绝不能删掉技能库里的实体内容"
    );
    assert!(list_installed(&store, &file, &pid).unwrap().is_empty());

    // 悬空：装好后把技能库里的技能删掉，UI 必须能识别出来
    install_into(&store, &file, "demo", &pid, None).unwrap();
    std::fs::remove_dir_all(store.join("demo")).unwrap();
    let list = list_installed(&store, &file, &pid).unwrap();
    assert_eq!(list.len(), 1, "悬空链接仍应被列出，否则用户看不到残留");
    assert_eq!(list[0].state, "broken");
}

/// 项目列表上显示的技能数必须和点进去看到的条目数**完全一致**。
///
/// 这是计数命令存在的全部理由，所以直接拿 `list_installed` 的结果做基准逐项比：
/// 一旦两边的判定口径分家（比如这里多要求了 is_dir），用户看到的数字就会对不上，
/// 而且这种错没有任何报错，只有「数字不对」。
#[cfg(windows)]
#[test]
fn project_skill_counts_match_the_installed_list() {
    let (base, store, file, pid) = setup_project_env("counts");
    make_skill(&store, "a");
    make_skill(&store, "b");
    install_into(&store, &file, "a", &pid, None).unwrap();
    install_into(&store, &file, "b", &pid, None).unwrap();
    // 再加一个项目自带（真实目录）与一个悬空链接
    let skills_dir = base.join("proj").join(".agents").join("skills");
    std::fs::create_dir_all(skills_dir.join("project-own")).unwrap();
    std::fs::write(
        skills_dir.join("project-own").join("SKILL.md"),
        "---\nname: Project Own\ndescription: 手写的技能\n---\n",
    )
    .unwrap();
    // 装好再把库里的实体删掉 → 项目里留下一个悬空链接
    make_skill(&store, "gone");
    install_into(&store, &file, "gone", &pid, None).expect("悬空用例的技能应装得上");
    std::fs::remove_dir_all(store.join("gone")).unwrap();

    let listed = list_installed(&store, &file, &pid).unwrap();
    let counts = count_project_skills(&file, &store);
    let c = counts
        .iter()
        .find(|c| c.project_id == pid)
        .expect("项目应出现在计数里");

    let alive = listed.iter().filter(|e| e.state != "broken").count();
    let linked = listed.iter().filter(|e| e.state == "linked").count();
    let local = listed.iter().filter(|e| e.state == "local").count();
    let broken = listed.iter().filter(|e| e.state == "broken").count();
    assert_eq!(
        (c.total, c.linked, c.local, c.broken),
        (alive, linked, local, broken)
    );
    assert_eq!((c.total, c.linked, c.local, c.broken), (3, 2, 1, 1));
}

/// 项目自带的真实目录也要被列出来（state=`local`），名字取自它自己的 SKILL.md。
/// 不列出来的话它在 UI 上完全隐形，用户既看不到也清不掉。
#[cfg(windows)]
#[test]
fn list_includes_local_real_directory_skills() {
    let (base, store, file, pid) = setup_project_env("local-listed");
    make_skill(&store, "from-library");
    let skills_dir = base.join("proj").join(".agents").join("skills");
    let real = skills_dir.join("project-own");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(
        real.join("SKILL.md"),
        "---\nname: Project Own\ndescription: 手写的技能\n---\n",
    )
    .unwrap();

    let list = list_installed(&store, &file, &pid).unwrap();
    assert_eq!(list.len(), 1, "项目自带技能应出现在列表里");
    assert_eq!(list[0].state, "local");
    assert_eq!(list[0].skill_id, "project-own");
    assert_eq!(list[0].name, "Project Own", "名称应读它自己的 SKILL.md");
    assert_eq!(list[0].description, "手写的技能");
}

/// 同名覆盖：项目里已有同名**真实目录**时允许覆盖 —— 删掉旧目录、建 junction。
///
/// 不可逆，所以 UI 必须在应用前把「会被替换的真实目录」列给用户确认。
#[cfg(windows)]
#[test]
fn install_overwrites_real_directory() {
    let (base, store, file, pid) = setup_project_env("overwrite");
    make_skill(&store, "demo");
    let skills_dir = base.join("proj").join(".agents").join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();
    let real = skills_dir.join("demo");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("MY_FILE.txt"), "old").unwrap();

    let outcome = install_into(&store, &file, "demo", &pid, None).expect("同名真实目录应被覆盖");
    let link = project_root(&file, &pid)
        .join(".agents")
        .join("skills")
        .join("demo");
    assert_eq!(
        outcome.replaced,
        vec![".agents".to_string()],
        "应报告被覆盖的目录"
    );
    assert!(
        link.join("SKILL.md").is_file(),
        "应变成指向技能库的 junction"
    );
    assert!(
        !real.join("MY_FILE.txt").exists(),
        "旧目录内容随覆盖一并移除"
    );
}

/// 卸载同理：真实目录与链接同等对待，整个删除。
#[cfg(windows)]
#[test]
fn uninstall_deletes_real_directory() {
    let (base, store, file, pid) = setup_project_env("uninstall-real");
    make_skill(&store, "demo");
    let skills_dir = base.join("proj").join(".agents").join("skills");
    std::fs::create_dir_all(skills_dir.join("demo")).unwrap();
    std::fs::write(skills_dir.join("demo").join("MY_FILE.txt"), "bye").unwrap();

    uninstall_from(&file, "demo", &pid, None).expect("真实目录应被删除");
    assert!(!skills_dir.join("demo").exists(), "目录应整个删掉");
    // 关键：技能库本体不能被牵连（只有链接才该这样，真实目录不会碰到技能库）
    assert!(
        store.join("demo").join("SKILL.md").is_file(),
        "技能库本体不受影响"
    );
}

/// 技能库与项目路径含空格时也要能装。
///
/// `create_junction` 走 `Command::args()` 而不是拼命令字符串，Rust 会自行正确引用，
/// 所以这里不需要额外加引号 —— 这条测试把该结论锁住，防止将来"顺手补引号"反而弄坏。
#[cfg(windows)]
#[test]
fn install_works_with_spaces_in_paths() {
    let base = temp_dir("with space");
    let store = base.join("my skills");
    std::fs::create_dir_all(&store).unwrap();
    make_skill(&store, "demo");
    let proj = base.join("my project");
    std::fs::create_dir_all(&proj).unwrap();
    let file = base.join("projects.json");
    let p = add_project_at(&file, &proj.to_string_lossy()).unwrap();

    install_into(&store, &file, "demo", &p.id, None).expect("含空格路径应能安装");
    assert!(project_root(&file, &p.id)
        .join(".agents")
        .join("skills")
        .join("demo")
        .join("SKILL.md")
        .is_file());
}
